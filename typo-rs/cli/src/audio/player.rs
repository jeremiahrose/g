//! Audio playback

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const SOURCE_SAMPLE_RATE: u32 = 24000; // Audio from OpenAI is 24kHz
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

        // Get the default config
        let supported_config = device
            .default_output_config()
            .context("Failed to get default output config")?;

        tracing::info!(
            "Default output config: {:?} channels, {:?} Hz, format: {:?}",
            supported_config.channels(),
            supported_config.sample_rate().0,
            supported_config.sample_format()
        );

        // Use the device's native sample rate
        let config = StreamConfig {
            channels: TARGET_CHANNELS.min(supported_config.channels()),
            sample_rate: supported_config.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };

        tracing::info!(
            "Using config: {} channels, {} Hz (source: {} Hz)",
            config.channels,
            config.sample_rate.0,
            SOURCE_SAMPLE_RATE
        );

        let output_rate = config.sample_rate.0;
        let needs_resampling = output_rate != SOURCE_SAMPLE_RATE;

        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let queue_clone = Arc::clone(&queue);

        // Build stream based on the sample format
        let stream = match supported_config.sample_format() {
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
        tokio::spawn(async move {
            while let Some(audio_bytes) = rx.recv().await {
                // Convert bytes to i16 samples
                let samples: Vec<i16> = audio_bytes
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();

                // Resample if needed
                let resampled = if needs_resampling {
                    resample_i16(&samples, SOURCE_SAMPLE_RATE, output_rate)
                } else {
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
