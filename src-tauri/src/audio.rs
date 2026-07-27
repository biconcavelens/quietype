use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Handle to an in-progress recording. The cpal stream lives on its own
/// thread (cpal::Stream isn't Send on Windows/WASAPI), so this handle only
/// carries channel endpoints, which are trivially Send + Sync.
pub struct RecordingHandle {
    stop_tx: mpsc::Sender<()>,
    result_rx: mpsc::Receiver<Vec<f32>>,
}

/// Starts capturing from the default input device.
///
/// `level_tx` receives a 0..1 loudness value per audio callback so the UI can
/// draw a live waveform. It is dropped when capture ends, which closes the
/// channel and lets the consumer's loop exit on its own.
pub fn start_recording(level_tx: mpsc::Sender<f32>) -> Result<RecordingHandle, String> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (result_tx, result_rx) = mpsc::channel::<Vec<f32>>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    thread::spawn(move || {
        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(d) => d,
            None => {
                let _ = ready_tx.send(Err("No microphone found.".to_string()));
                return;
            }
        };
        let supported = match device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };

        let sample_rate = supported.sample_rate();
        let channels = supported.channels();
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();

        let buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
        let buffer_cb = buffer.clone();
        let err_fn = |err| eprintln!("[quietype] audio stream error: {err}");

        let stream_result = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let _ = level_tx.send(level_of(data));
                    buffer_cb.lock().unwrap().extend_from_slice(data);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut buf = buffer_cb.lock().unwrap();
                    let start = buf.len();
                    buf.extend(data.iter().map(|s| *s as f32 / i16::MAX as f32));
                    let _ = level_tx.send(level_of(&buf[start..]));
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let mut buf = buffer_cb.lock().unwrap();
                    let start = buf.len();
                    buf.extend(data.iter().map(|s| (*s as f32 - 32768.0) / 32768.0));
                    let _ = level_tx.send(level_of(&buf[start..]));
                },
                err_fn,
                None,
            ),
            other => {
                let _ = ready_tx.send(Err(format!("Unsupported audio format: {other:?}")));
                return;
            }
        };

        let stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };

        if let Err(e) = stream.play() {
            let _ = ready_tx.send(Err(e.to_string()));
            return;
        }
        let _ = ready_tx.send(Ok(()));

        // Block this dedicated thread until told to stop; dropping `stream`
        // (end of scope) tears down the cpal stream.
        let _ = stop_rx.recv();
        drop(stream);

        let raw = buffer.lock().unwrap().clone();
        let mono = downmix(&raw, channels);
        let resampled = resample_linear(&mono, sample_rate, TARGET_SAMPLE_RATE);
        let _ = result_tx.send(resampled);
    });

    ready_rx.recv().map_err(|e| e.to_string())??;
    Ok(RecordingHandle {
        stop_tx,
        result_rx,
    })
}

impl RecordingHandle {
    /// Stops the stream and returns mono 16kHz f32 samples, ready for whisper.
    pub fn stop(self) -> Vec<f32> {
        let _ = self.stop_tx.send(());
        self.result_rx.recv().unwrap_or_default()
    }
}

/// RMS loudness, scaled into a 0..1 range that looks reasonable on a meter.
fn level_of(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    let rms = (sum_sq / samples.len() as f32).sqrt();
    (rms * 4.0).clamp(0.0, 1.0)
}

fn downmix(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let channels = channels as usize;
    samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

// ponytail: naive linear resample. Good enough for 16kHz speech transcription;
// swap for a proper sinc resampler (e.g. `rubato`) if quality suffers on
// unusual input sample rates.
fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (samples.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src_pos = i as f64 * ratio;
            let idx = src_pos as usize;
            let frac = (src_pos - idx as f64) as f32;
            let a = samples.get(idx).copied().unwrap_or(0.0);
            let b = samples.get(idx + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_zero_and_loud_is_capped() {
        assert_eq!(level_of(&[]), 0.0);
        assert_eq!(level_of(&[0.0; 64]), 0.0);
        assert_eq!(level_of(&[1.0; 64]), 1.0);
        assert!(level_of(&[0.05; 64]) > 0.0);
    }

    #[test]
    fn downmix_averages_channel_pairs() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(downmix(&[1.0, 2.0], 1), vec![1.0, 2.0]);
    }

    #[test]
    fn resample_halves_length_when_rate_halves() {
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        assert_eq!(resample_linear(&input, 32_000, 16_000).len(), 50);
        assert_eq!(resample_linear(&input, 16_000, 16_000).len(), 100);
    }
}
