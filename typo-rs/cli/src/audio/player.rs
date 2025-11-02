//! Audio playback

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use tokio::sync::Mutex;

const SAMPLE_RATE: u32 = 24000;
const CHANNELS: u16 = 1;

pub struct AudioPlayer {
    queue: Arc<Mutex<Vec<i16>>>,
    _stream: Option<cpal::Stream>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No output device available")?;

        let config = cpal::StreamConfig {
            channels: CHANNELS,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        let queue = Arc::new(Mutex::new(Vec::new()));
        let queue_clone = Arc::clone(&queue);

        let stream = device.build_output_stream(
            &config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                let mut queue = queue_clone.try_lock();
                if let Ok(ref mut queue) = queue {
                    let len = data.len().min(queue.len());
                    data[..len].copy_from_slice(&queue[..len]);
                    queue.drain(..len);

                    // Fill rest with silence
                    for sample in data[len..].iter_mut() {
                        *sample = 0;
                    }
                }
            },
            move |err| {
                tracing::error!("Audio playback error: {}", err);
            },
            None,
        )?;

        stream.play()?;

        Ok(Self {
            queue,
            _stream: Some(stream),
        })
    }

    pub async fn play(&self, audio_bytes: &[u8]) -> Result<()> {
        // Convert bytes to i16 samples
        let samples: Vec<i16> = audio_bytes
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let mut queue = self.queue.lock().await;
        queue.extend_from_slice(&samples);

        Ok(())
    }
}
