//! Audio playback

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc, Mutex};

const SOURCE_SAMPLE_RATE: u32 = 24000; // Audio from OpenAI is 24kHz
const TARGET_CHANNELS: u16 = 1;

const RESAMPLER_CHUNK_SIZE: usize = 480; // 20ms at 24kHz

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

/// Helper to manage buffering and resampling for playback
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
                    tracing::warn!("Playback resampling failed: {}", e);
                }
            }
        }

        output
    }
}

pub struct AudioPlayer {
    _stream: Option<cpal::Stream>,
}

impl AudioPlayer {
    pub fn new() -> Result<(Self, mpsc::UnboundedSender<Vec<u8>>)> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No output device available")?;

        tracing::info!("Output device: {}", device.name().unwrap_or_default());

        // Try to find if device supports 24kHz natively
        let supported_configs: Vec<_> = device
            .supported_output_configs()
            .context("Failed to get supported output configs")?
            .collect();

        tracing::info!("Device reports {} supported output configurations:", supported_configs.len());
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
            if config_range.min_sample_rate().0 <= SOURCE_SAMPLE_RATE
                && config_range.max_sample_rate().0 >= SOURCE_SAMPLE_RATE
                && config_range.channels() >= TARGET_CHANNELS
            {
                found_24khz = true;
                tracing::info!("Device supports 24kHz natively!");
                break;
            }
        }

        let default_config = device
            .default_output_config()
            .context("Failed to get default output config")?;

        let sample_format = default_config.sample_format();

        let config = if found_24khz {
            // Use 24kHz directly - no resampling needed!
            tracing::info!("Using native 24kHz playback - no resampling");
            StreamConfig {
                channels: TARGET_CHANNELS,
                sample_rate: SampleRate(SOURCE_SAMPLE_RATE),
                buffer_size: cpal::BufferSize::Default,
            }
        } else {
            tracing::info!(
                "Device doesn't support 24kHz output, using {} Hz and will resample",
                default_config.sample_rate().0
            );

            StreamConfig {
                channels: TARGET_CHANNELS.min(default_config.channels()),
                sample_rate: default_config.sample_rate(),
                buffer_size: cpal::BufferSize::Default,
            }
        };

        tracing::info!(
            "Using config: {} channels, {} Hz (source: {} Hz)",
            config.channels,
            config.sample_rate.0,
            SOURCE_SAMPLE_RATE
        );

        let output_rate = config.sample_rate.0;
        let needs_resampling = output_rate != SOURCE_SAMPLE_RATE;

        // Create resampler if needed
        let resampler = if needs_resampling {
            tracing::info!("Creating high-quality resampler for playback: {} Hz -> {} Hz", SOURCE_SAMPLE_RATE, output_rate);
            let resampler_inner = create_resampler(SOURCE_SAMPLE_RATE, output_rate)?;
            let buffer = ResamplerBuffer::new(resampler_inner);
            Some(Arc::new(StdMutex::new(buffer)))
        } else {
            None
        };

        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let queue_clone = Arc::clone(&queue);

        // Build stream based on the sample format
        let stream = match sample_format {
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let mut queue = queue_clone.try_lock();
                    if let Ok(ref mut queue) = queue {
                        for sample in data.iter_mut() {
                            *sample = queue.pop_front().unwrap_or(0);
                        }
                    }
                },
                move |err| {
                    tracing::error!("Audio playback error: {}", err);
                },
                None,
            )?,
            SampleFormat::F32 => {
                let queue_clone2 = Arc::clone(&queue);
                device.build_output_stream(
                    &config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        let mut queue = queue_clone2.try_lock();
                        if let Ok(ref mut queue) = queue {
                            for sample in data.iter_mut() {
                                let i16_sample = queue.pop_front().unwrap_or(0);
                                // Convert i16 to f32
                                *sample = i16_sample as f32 / 32768.0;
                            }
                        }
                    },
                    move |err| {
                        tracing::error!("Audio playback error: {}", err);
                    },
                    None,
                )?
            }
            format => {
                return Err(anyhow::anyhow!("Unsupported sample format: {:?}", format));
            }
        };

        stream.play()?;

        // Create channel for receiving audio
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // Spawn task to process incoming audio
        let resampler_task = resampler.clone();
        tokio::spawn(async move {
            while let Some(audio_bytes) = rx.recv().await {
                // Convert bytes to i16 samples
                let samples: Vec<i16> = audio_bytes
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();

                // Resample if needed
                let resampled = if let Some(ref resampler_arc) = resampler_task {
                    // High-quality resampling with buffering
                    if let Ok(mut resampler_buf) = resampler_arc.lock() {
                        resampler_buf.process(&samples)
                    } else {
                        // Lock poisoned - skip resampling
                        samples
                    }
                } else {
                    // No resampling needed
                    samples
                };

                let mut queue = queue.lock().await;
                queue.extend(resampled);
            }
        });

        Ok((
            Self {
                _stream: Some(stream),
            },
            tx,
        ))
    }
}
