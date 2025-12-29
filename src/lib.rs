mod utils;

use rayon::prelude::*;
use wasm_bindgen::prelude::*;

pub use wasm_bindgen_rayon::init_thread_pool;

#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
    #[wasm_bindgen(js_namespace = Date)]
    fn now() -> f64;
}

#[wasm_bindgen]
pub fn greet() {
    alert("Hello, wasm!");
}

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum SingleAudioFileType {
    Wav,
    Mpeg,
    Ogg,
}

#[wasm_bindgen]
pub struct SingleAudioFile {
    #[wasm_bindgen(getter_with_clone)]
    pub bytes: Vec<u8>,
    pub r#type: SingleAudioFileType,
}

#[wasm_bindgen]
impl SingleAudioFile {
    pub fn new(bytes: Vec<u8>, r#type: SingleAudioFileType) -> Self {
        Self { bytes, r#type }
    }
}

fn create_wav_container(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let mut wav = Vec::new();
    let data_size = (samples.len() * 2) as u32; // 2 bytes per sample (i16)

    // RIFF Header
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    // fmt chunk
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes()); // Hardcoded Stereo
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());

    // data chunk
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());

    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let s = (clamped * i16::MAX as f32) as i16;
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

struct AudioCombinerSingleFile {
    samples: Vec<f32>,
}
#[wasm_bindgen]
pub struct AudioCombiner {
    files: Vec<AudioCombinerSingleFile>,
}

#[wasm_bindgen]
impl AudioCombiner {
    pub fn new(files: Vec<SingleAudioFile>) -> Result<AudioCombiner, String> {
        // 1. Parallel Decoding using Rayon
        // par_iter() allows us to process files across multiple Web Workers
        let processed_files: Result<Vec<AudioCombinerSingleFile>, String> = files
            .into_par_iter()
            .map(|file| {
                let mut decoded_samples = Vec::new();
                let src = std::io::Cursor::new(file.bytes);
                let mss =
                    symphonia::core::io::MediaSourceStream::new(Box::new(src), Default::default());

                let mut hint = symphonia::core::probe::Hint::new();
                match file.r#type {
                    SingleAudioFileType::Wav => {
                        hint.with_extension("wav");
                    }
                    SingleAudioFileType::Mpeg => {
                        hint.with_extension("mp3");
                    }
                    SingleAudioFileType::Ogg => {
                        hint.with_extension("ogg");
                    }
                }

                let probed = symphonia::default::get_probe()
                    .format(&hint, mss, &Default::default(), &Default::default())
                    .map_err(|e| e.to_string())?;

                let mut format = probed.format;
                let track = format.default_track().ok_or("No supported audio track")?;
                let mut decoder = symphonia::default::get_codecs()
                    .make(&track.codec_params, &Default::default())
                    .map_err(|e| e.to_string())?;

                let mut sample_buf = None;

                while let Ok(packet) = format.next_packet() {
                    let decoded = decoder.decode(&packet).map_err(|e| e.to_string())?;
                    let spec = *decoded.spec();
                    let num_channels = spec.channels.count();

                    let buf = sample_buf.get_or_insert_with(|| {
                        symphonia::core::audio::SampleBuffer::<f32>::new(
                            decoded.capacity() as u64,
                            spec,
                        )
                    });
                    buf.copy_interleaved_ref(decoded);

                    for frame in buf.samples().chunks(num_channels) {
                        if num_channels == 1 {
                            decoded_samples.push(frame[0]);
                            decoded_samples.push(frame[0]);
                        } else {
                            decoded_samples.push(frame[0]);
                            decoded_samples.push(frame[1]);
                        }
                    }
                }
                Ok(AudioCombinerSingleFile {
                    samples: decoded_samples,
                })
            })
            .collect();

        Ok(AudioCombiner {
            files: processed_files?,
        })
    }

    pub fn combine(&self, volumes: Vec<u8>) -> Result<SingleAudioFile, String> {
        let target_sample_rate = 44100u32;

        let max_len = self
            .files
            .iter()
            .map(|f| f.samples.len())
            .max()
            .unwrap_or(0);

        // Convert volumes to f32 once
        let vol_factors: Vec<f32> = volumes.iter().map(|&v| v as f32 / 100.0).collect();

        // 2. Parallel Mixing
        // We create the master buffer by iterating over the indices in parallel.
        // Each thread works on a "chunk" of the master buffer.
        let master_buffer: Vec<f32> = (0..max_len)
            .into_par_iter()
            .map(|i| {
                let mut sum = 0.0f32;
                for (file_idx, file) in self.files.iter().enumerate() {
                    if let Some(&sample) = file.samples.get(i) {
                        let vol = *vol_factors.get(file_idx).unwrap_or(&1.0);
                        sum += sample * vol;
                    }
                }
                sum
            })
            .collect();

        Ok(SingleAudioFile {
            bytes: create_wav_container(&master_buffer, target_sample_rate),
            r#type: SingleAudioFileType::Wav,
        })
    }
}

#[wasm_bindgen]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
