//! Audio recording

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const TARGET_SAMPLE_RATE: u32 = 24000;
const TARGET_CHANNELS: u16 = 1;

/// Linear interpolation resampling for i16 audio
/// Provides better quality than nearest-neighbor by interpolating between samples
fn resample_i16(input: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if from_rate == to_rate {
        return input.to_vec();
    }

    let ratio = from_rate as f32 / to_rate as f32;
    let output_len = (input.len() as f32 / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_pos = i as f32 * ratio;
        let src_idx = src_pos as usize;
        let frac = src_pos - src_idx as f32;

        if src_idx + 1 < input.len() {
            // Linear interpolation between two adjacent samples
            let sample = input[src_idx] as f32 * (1.0 - frac) + input[src_idx + 1] as f32 * frac;
            output.push(sample as i16);
        } else if src_idx < input.len() {
            output.push(input[src_idx]);
        }
    }

    output
}

pub struct AudioRecorder {
    audio_tx: Arc<Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>>,
    _stream: Option<cpal::Stream>,
}

impl AudioRecorder {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No input device available")?;

        tracing::info!("Input device: {}", device.name().unwrap_or_default());

        // Try to get the default config and adapt it
        let supported_config = device
            .default_input_config()
            .context("Failed to get default input config")?;

        tracing::info!(
            "Default input config: {:?} channels, {:?} Hz, format: {:?}",
            supported_config.channels(),
            supported_config.sample_rate().0,
            supported_config.sample_format()
        );

        // Use the device's native sample rate (we'll resample if needed)
        let config = StreamConfig {
            channels: TARGET_CHANNELS.min(supported_config.channels()),
            sample_rate: supported_config.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };

        tracing::info!(
            "Using config: {} channels, {} Hz (target: {} Hz)",
            config.channels,
            config.sample_rate.0,
            TARGET_SAMPLE_RATE
        );

        let source_rate = config.sample_rate.0;
        let needs_resampling = source_rate != TARGET_SAMPLE_RATE;

        let audio_tx: Arc<Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>> =
            Arc::new(Mutex::new(None));
        let audio_tx_clone = Arc::clone(&audio_tx);

        // Build stream based on the sample format
        let stream = match supported_config.sample_format() {
            SampleFormat::I16 => {
                if needs_resampling {
                    device.build_input_stream(
                        &config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if let Ok(tx_guard) = audio_tx_clone.try_lock() {
                                if let Some(ref tx) = *tx_guard {
                                    // Resample from source_rate to TARGET_SAMPLE_RATE
                                    let resampled = resample_i16(data, source_rate, TARGET_SAMPLE_RATE);

                                    // Convert i16 samples to bytes
                                    let bytes: Vec<u8> = resampled
                                        .iter()
                                        .flat_map(|&sample| sample.to_le_bytes())
                                        .collect();

                                    if let Err(e) = tx.send(bytes) {
                                        tracing::error!("Failed to send audio chunk: {}", e);
                                    }
                                }
                            }
                        },
                        move |err| {
                            tracing::error!("Audio recording error: {}", err);
                        },
                        None,
                    )?
                } else {
                    device.build_input_stream(
                        &config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if let Ok(tx_guard) = audio_tx_clone.try_lock() {
                                if let Some(ref tx) = *tx_guard {
                                    // Convert i16 samples to bytes
                                    let bytes: Vec<u8> = data
                                        .iter()
                                        .flat_map(|&sample| sample.to_le_bytes())
                                        .collect();

                                    if let Err(e) = tx.send(bytes) {
                                        tracing::error!("Failed to send audio chunk: {}", e);
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
            }
            SampleFormat::F32 => {
                let audio_tx_clone2 = Arc::clone(&audio_tx);
                let callback_count = Arc::new(Mutex::new(0u64));
                if needs_resampling {
                    let callback_count_clone = Arc::clone(&callback_count);
                    device.build_input_stream(
                        &config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if let Ok(mut count) = callback_count_clone.try_lock() {
                                *count += 1;
                                if *count % 50 == 0 {
                                    tracing::debug!("Audio callback called {} times", *count);
                                }
                            }

                            match audio_tx_clone2.try_lock() {
                                Ok(tx_guard) => {
                                    if let Some(ref tx) = *tx_guard {
                                        // Convert f32 to i16 first
                                        let i16_samples: Vec<i16> = data
                                            .iter()
                                            .map(|&sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16)
                                            .collect();

                                        // Resample
                                        let resampled = resample_i16(&i16_samples, source_rate, TARGET_SAMPLE_RATE);

                                        // Convert to bytes
                                        let bytes: Vec<u8> = resampled
                                            .iter()
                                            .flat_map(|&sample| sample.to_le_bytes())
                                            .collect();

                                        if let Err(e) = tx.send(bytes) {
                                            tracing::error!("Failed to send audio chunk: {}", e);
                                        } else {
                                            // Successfully sent
                                            if let Ok(count) = callback_count_clone.try_lock() {
                                                if *count % 100 == 0 {
                                                    tracing::debug!("Successfully sent audio chunk {}", *count);
                                                }
                                            }
                                        }
                                    } else {
                                        // Sender not ready yet
                                        if let Ok(count) = callback_count_clone.try_lock() {
                                            if *count % 100 == 1 {
                                                tracing::warn!("Audio callback running but sender not initialized (call {})", *count);
                                            }
                                        }
                                    }
                                }
                                Err(_) => {
                                    // Lock contention
                                    if let Ok(count) = callback_count_clone.try_lock() {
                                        if *count % 100 == 0 {
                                            tracing::warn!("Audio callback lock contention at call {}", *count);
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
                } else {
                    device.build_input_stream(
                        &config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if let Ok(tx_guard) = audio_tx_clone2.try_lock() {
                                if let Some(ref tx) = *tx_guard {
                                    // Convert f32 samples to i16 bytes
                                    let bytes: Vec<u8> = data
                                        .iter()
                                        .flat_map(|&sample| {
                                            let i16_sample = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                                            i16_sample.to_le_bytes()
                                        })
                                        .collect();

                                    if let Err(e) = tx.send(bytes) {
                                        tracing::error!("Failed to send audio chunk: {}", e);
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
        let mut audio_tx = self.audio_tx.lock().await;
        *audio_tx = Some(tx);
        rx
    }

    #[allow(dead_code)]
    pub async fn stop_recording(&self) {
        let mut audio_tx = self.audio_tx.lock().await;
        *audio_tx = None;
    }
}
