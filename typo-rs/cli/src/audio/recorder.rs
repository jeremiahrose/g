//! Audio recording

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::mpsc;

const TARGET_SAMPLE_RATE: u32 = 24000;
const TARGET_CHANNELS: u16 = 1;

const RESAMPLER_CHUNK_SIZE: usize = 480; // 20ms at 24kHz = good compromise

/// High-quality resampling using Rubato (sinc interpolation)
/// This matches the quality of PortAudio's internal resampling
fn create_resampler(from_rate: u32, to_rate: u32) -> Result<SincFixedIn<f32>> {
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    // Calculate input chunk size based on resampling ratio
    let ratio = from_rate as f64 / to_rate as f64;
    let input_chunk_size = (RESAMPLER_CHUNK_SIZE as f64 * ratio).ceil() as usize;

    SincFixedIn::<f32>::new(
        to_rate as f64 / from_rate as f64,
        2.0,
        params,
        input_chunk_size,
        1, // channels
    )
    .context("Failed to create resampler")
}

/// Helper to manage buffering and resampling
struct ResamplerBuffer {
    resampler: SincFixedIn<f32>,
    buffer: VecDeque<i16>,
    chunk_size: usize,
}

impl ResamplerBuffer {
    fn new(resampler: SincFixedIn<f32>) -> Self {
        let chunk_size = resampler.input_frames_next();
        Self {
            resampler,
            buffer: VecDeque::new(),
            chunk_size,
        }
    }

    /// Add samples to buffer and process when we have enough
    fn process(&mut self, input: &[i16]) -> Vec<i16> {
        // Add new samples to buffer
        self.buffer.extend(input.iter().copied());

        let mut output = Vec::new();

        // Process as many complete chunks as we have
        while self.buffer.len() >= self.chunk_size {
            // Take exactly chunk_size samples
            let chunk: Vec<i16> = self.buffer.drain(..self.chunk_size).collect();

            // Convert i16 to f32 in range [-1.0, 1.0]
            let input_f32: Vec<f32> = chunk
                .iter()
                .map(|&sample| sample as f32 / 32768.0)
                .collect();

            // Resample
            let waves_in = vec![input_f32];
            match self.resampler.process(&waves_in, None) {
                Ok(waves_out) => {
                    // Convert f32 back to i16
                    let resampled: Vec<i16> = waves_out[0]
                        .iter()
                        .map(|&sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16)
                        .collect();
                    output.extend_from_slice(&resampled);
                }
                Err(e) => {
                    tracing::warn!("Resampling failed: {}", e);
                }
            }
        }

        output
    }
}

pub struct AudioRecorder {
    audio_tx: Arc<StdMutex<Option<mpsc::UnboundedSender<Vec<u8>>>>>,
    _stream: Option<cpal::Stream>,
}

impl AudioRecorder {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No input device available")?;

        tracing::info!("Input device: {}", device.name().unwrap_or_default());

        // Try to find if device supports 24kHz natively
        let supported_configs: Vec<_> = device
            .supported_input_configs()
            .context("Failed to get supported input configs")?
            .collect();

        tracing::info!("Device reports {} supported input configurations:", supported_configs.len());
        for (i, config_range) in supported_configs.iter().enumerate() {
            tracing::info!(
                "  Config {}: {} channels, {} Hz - {} Hz, format: {:?}",
                i,
                config_range.channels(),
                config_range.min_sample_rate().0,
                config_range.max_sample_rate().0,
                config_range.sample_format()
            );
        }

        let mut found_24khz = false;
        for config_range in &supported_configs {
            if config_range.min_sample_rate().0 <= TARGET_SAMPLE_RATE
                && config_range.max_sample_rate().0 >= TARGET_SAMPLE_RATE
                && config_range.channels() >= TARGET_CHANNELS
            {
                found_24khz = true;
                tracing::info!("Device supports 24kHz natively!");
                break;
            }
        }

        let default_config = device
            .default_input_config()
            .context("Failed to get default input config")?;

        let sample_format = default_config.sample_format();

        let config = if found_24khz {
            // Use 24kHz directly - no resampling needed!
            tracing::info!("Using native 24kHz - no resampling");
            StreamConfig {
                channels: TARGET_CHANNELS,
                sample_rate: SampleRate(TARGET_SAMPLE_RATE),
                buffer_size: cpal::BufferSize::Default,
            }
        } else {
            tracing::info!(
                "Device doesn't support 24kHz, using {} Hz and will resample",
                default_config.sample_rate().0
            );

            StreamConfig {
                channels: TARGET_CHANNELS.min(default_config.channels()),
                sample_rate: default_config.sample_rate(),
                buffer_size: cpal::BufferSize::Default,
            }
        };

        tracing::info!(
            "Using config: {} channels, {} Hz (target: {} Hz)",
            config.channels,
            config.sample_rate.0,
            TARGET_SAMPLE_RATE
        );

        let source_rate = config.sample_rate.0;
        let needs_resampling = source_rate != TARGET_SAMPLE_RATE;

        // Create resampler if needed
        let resampler = if needs_resampling {
            tracing::info!("Creating high-quality resampler: {} Hz -> {} Hz", source_rate, TARGET_SAMPLE_RATE);
            let resampler_inner = create_resampler(source_rate, TARGET_SAMPLE_RATE)?;
            let buffer = ResamplerBuffer::new(resampler_inner);
            Some(Arc::new(StdMutex::new(buffer)))
        } else {
            None
        };

        let audio_tx: Arc<StdMutex<Option<mpsc::UnboundedSender<Vec<u8>>>>> =
            Arc::new(StdMutex::new(None));
        let audio_tx_clone = Arc::clone(&audio_tx);
        let resampler_clone = resampler.clone();

        // Build stream based on the sample format
        let stream = match sample_format {
            SampleFormat::I16 => {
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if let Ok(tx_guard) = audio_tx_clone.try_lock() {
                            if let Some(ref tx) = *tx_guard {
                                let samples = if let Some(ref resampler_arc) = resampler_clone {
                                    // High-quality resampling with buffering
                                    if let Ok(mut resampler_buf) = resampler_arc.lock() {
                                        resampler_buf.process(data)
                                    } else {
                                        // Lock poisoned - skip this chunk
                                        return;
                                    }
                                } else {
                                    // No resampling needed
                                    data.to_vec()
                                };

                                // Only send if we have output (buffer might not be full yet)
                                if !samples.is_empty() {
                                    // Convert i16 samples to bytes
                                    let bytes: Vec<u8> = samples
                                        .iter()
                                        .flat_map(|&sample| sample.to_le_bytes())
                                        .collect();

                                    if let Err(e) = tx.send(bytes) {
                                        tracing::error!("Failed to send audio chunk: {}", e);
                                    }
                                }
                            }
                        }
                    },
                    move |err| {
                        tracing::error!("Audio recording error: {}", err);
                    },
                    None,
                )?
            }
            SampleFormat::F32 => {
                let audio_tx_clone2 = Arc::clone(&audio_tx);
                let resampler_clone2 = resampler.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if let Ok(tx_guard) = audio_tx_clone2.try_lock() {
                            if let Some(ref tx) = *tx_guard {
                                // Convert f32 to i16 first
                                let i16_samples: Vec<i16> = data
                                    .iter()
                                    .map(|&sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16)
                                    .collect();

                                let samples = if let Some(ref resampler_arc) = resampler_clone2 {
                                    // High-quality resampling with buffering
                                    if let Ok(mut resampler_buf) = resampler_arc.lock() {
                                        resampler_buf.process(&i16_samples)
                                    } else {
                                        // Lock poisoned - skip this chunk
                                        return;
                                    }
                                } else {
                                    // No resampling needed
                                    i16_samples
                                };

                                // Only send if we have output
                                if !samples.is_empty() {
                                    // Convert to bytes
                                    let bytes: Vec<u8> = samples
                                        .iter()
                                        .flat_map(|&sample| sample.to_le_bytes())
                                        .collect();

                                    if let Err(e) = tx.send(bytes) {
                                        tracing::error!("Failed to send audio chunk: {}", e);
                                    }
                                }
                            }
                        }
                    },
                    move |err| {
                        tracing::error!("Audio recording error: {}", err);
                    },
                    None,
                )?
            }
            format => {
                return Err(anyhow::anyhow!("Unsupported sample format: {:?}", format));
            }
        };

        stream.play()?;

        Ok(Self {
            audio_tx,
            _stream: Some(stream),
        })
    }

    pub async fn start_recording(&self) -> mpsc::UnboundedReceiver<Vec<u8>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut audio_tx = self.audio_tx.lock().unwrap();
        *audio_tx = Some(tx);
        rx
    }

    #[allow(dead_code)]
    pub async fn stop_recording(&self) {
        let mut audio_tx = self.audio_tx.lock().unwrap();
        *audio_tx = None;
    }
}
