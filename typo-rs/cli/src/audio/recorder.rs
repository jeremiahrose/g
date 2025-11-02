//! Audio recording

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const SAMPLE_RATE: u32 = 24000;
const CHANNELS: u16 = 1;

pub struct AudioRecorder {
    audio_tx: Arc<Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>>,
    _stream: cpal::Stream,
}

impl AudioRecorder {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No input device available")?;

        let config = cpal::StreamConfig {
            channels: CHANNELS,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        let audio_tx = Arc::new(Mutex::new(None));
        let audio_tx_clone = Arc::clone(&audio_tx);

        let stream = device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                if let Ok(tx_guard) = audio_tx_clone.try_lock() {
                    if let Some(ref tx) = *tx_guard {
                        // Convert i16 samples to bytes
                        let bytes: Vec<u8> = data
                            .iter()
                            .flat_map(|&sample| sample.to_le_bytes())
                            .collect();

                        let _ = tx.send(bytes);
                    }
                }
            },
            move |err| {
                tracing::error!("Audio recording error: {}", err);
            },
            None,
        )?;

        stream.play()?;

        Ok(Self {
            audio_tx,
            _stream: stream,
        })
    }

    pub async fn start_recording(&self) -> mpsc::UnboundedReceiver<Vec<u8>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut audio_tx = self.audio_tx.lock().await;
        *audio_tx = Some(tx);
        rx
    }

    pub async fn stop_recording(&self) {
        let mut audio_tx = self.audio_tx.lock().await;
        *audio_tx = None;
    }
}
