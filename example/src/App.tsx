import React from "react";
import "./App.css";
import * as Comlink from "comlink";
import Worker from "./worker.ts?worker";
import type { Api } from "./worker.ts";
import * as wasm from "wasm";

const worker = new Worker();
const api: Api = Comlink.wrap(worker);

interface AudioFile {
  id: string;
  name: string;
  duration: string;
  volume: number;
}

function assertNever(value: never) {
  throw new Error("Unexpected value: " + value);
}

let ran = false;

// function usePrevious<T>(value: T): T | undefined {
//   const ref = React.useRef<T>();
//   React.useEffect(() => {
//     ref.current = value;
//   }, [value]);
//   return ref.current;
// }

function App() {
  const [files, setFiles] = React.useState<AudioFile[]>([]);

  React.useEffect(() => {
    if (ran) {
      return;
    }
    ran = true;
    (async () => {
      await api.init();
    })();
  }, []);

  // Persist the WASM instance and the Audio element
  const audioCombinerRef =
    React.useRef<Awaited<ReturnType<typeof api.createCombiner>>>(null);
  const fileInputRef = React.useRef<HTMLInputElement>(null);

  const audioRef = React.useRef<HTMLAudioElement>(null); // Visible Player
  const loaderRef = React.useRef<HTMLAudioElement>(null); // Hidden Probe
  const isSwappingRef = React.useRef(false);
  const pendingUpdateRef = React.useRef<{
    bytes: Uint8Array<ArrayBufferLike>;
    type: wasm.SingleAudioFileType;
  } | null>(null);

  const updateAudioSource = (file: {
    bytes: Uint8Array<ArrayBufferLike>;
    type: wasm.SingleAudioFileType;
  }) => {
    // 1. If we are already busy swapping, queue the latest file and exit
    if (isSwappingRef.current) {
      pendingUpdateRef.current = file;
      return;
    }

    isSwappingRef.current = true;

    let audioType: string = "";
    switch (file.type) {
      case wasm.SingleAudioFileType.Wav:
        audioType = "audio/wav";
        break;
      case wasm.SingleAudioFileType.Mpeg:
        audioType = "audio/mpeg";
        break;
      case wasm.SingleAudioFileType.Ogg:
        audioType = "audio/ogg";
        break;
      default:
        assertNever(file.type);
        break;
    }
    const blob = new Blob([file.bytes as BlobPart], { type: audioType });
    const newUrl = URL.createObjectURL(blob);

    const mainAudio = audioRef.current;
    const loaderAudio = loaderRef.current;

    if (mainAudio && loaderAudio) {
      // 2. Load the file into the hidden "loader" element first
      loaderAudio.src = newUrl;
      loaderAudio.load();

      const handleReady = () => {
        // 3. Sync state from visible player
        const currentTime = mainAudio.currentTime;
        const isPlaying = !mainAudio.paused;

        // 4. Swap the URL to the main player
        if (mainAudio.src) {
          URL.revokeObjectURL(mainAudio.src);
        }
        mainAudio.src = newUrl;

        // We must call load() and restore state on the main player
        mainAudio.load();
        mainAudio.currentTime = currentTime;
        if (isPlaying)
          mainAudio.play().catch((e) => console.error("Playback error:", e));

        // 5. Cleanup and check for pending updates
        isSwappingRef.current = false;
        loaderAudio.removeEventListener("canplaythrough", handleReady);

        if (pendingUpdateRef.current) {
          const nextFile = pendingUpdateRef.current;
          pendingUpdateRef.current = null;
          updateAudioSource(nextFile);
        }
      };

      loaderAudio.addEventListener("canplaythrough", handleReady);
    }
  };

  const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const selectedFiles = Array.from(e.target.files ?? []);
    if (selectedFiles.length === 0) return;

    // Initialize/Reset the merger instance
    const audioCombiner = await api.createCombiner(
      await Promise.all(
        selectedFiles.map(async (file) => {
          return {
            bytes: new Uint8Array(await file.arrayBuffer()),
            type:
              {
                "audio/mpeg": wasm.SingleAudioFileType.Mpeg,
                "audio/wav": wasm.SingleAudioFileType.Wav,
              }[file.type] ?? wasm.SingleAudioFileType.Mpeg,
          };
        })
      )
    );

    const newFiles = await Promise.all(
      selectedFiles.map(async (file) => {
        const duration = await getAudioDuration(file);
        return {
          id: Math.random().toString(36).substring(2, 9),
          name: file.name,
          duration: duration,
          volume: 100,
        };
      })
    );

    audioCombinerRef.current = audioCombiner;
    setFiles(newFiles);

    // Initial combine
    const initialVolumes = new Uint8Array(newFiles.map(() => 100));
    const combinedFile = await audioCombinerRef.current.combine(initialVolumes);
    updateAudioSource(combinedFile);
  };

  const updateVolume = async (id: string, value: number) => {
    // 1. Update the state for the UI
    const updatedFiles = files.map((f) =>
      f.id === id ? { ...f, volume: value } : f
    );
    setFiles(updatedFiles);

    // 2. Use the persisted merger to get new audio data
    if (audioCombinerRef.current) {
      const volumes = new Uint8Array(updatedFiles.map((f) => f.volume));
      const combinedFile = await audioCombinerRef.current.combine(volumes);
      updateAudioSource(combinedFile);
    }
  };

  const getAudioDuration = async (file: File): Promise<string> => {
    const buffer = await file.arrayBuffer();
    const view = new DataView(buffer);
    let offset = 0;

    if (
      view.getUint8(0) === 0x49 &&
      view.getUint8(1) === 0x44 &&
      view.getUint8(2) === 0x33
    ) {
      const size =
        (view.getUint8(6) << 21) |
        (view.getUint8(7) << 14) |
        (view.getUint8(8) << 7) |
        view.getUint8(9);
      offset = size + 10;
    }

    let durationSeconds = 0;
    while (offset < view.byteLength - 4) {
      if (
        view.getUint8(offset) === 0xff &&
        (view.getUint8(offset + 1) & 0xe0) === 0xe0
      ) {
        const byte2 = view.getUint8(offset + 2);
        const bitrates = [
          0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
        ];
        const bitrate = bitrates[(byte2 >> 4) & 15];
        if (bitrate > 0) durationSeconds = (file.size * 8) / (bitrate * 1000);
        break;
      }
      offset++;
    }

    const h = Math.floor(durationSeconds / 3600)
      .toString()
      .padStart(2, "0");
    const m = Math.floor((durationSeconds % 3600) / 60)
      .toString()
      .padStart(2, "0");
    const s = Math.floor(durationSeconds % 60)
      .toString()
      .padStart(2, "0");
    return `${h}:${m}:${s}`;
  };

  const reset = () => {
    setFiles([]);
    audioCombinerRef.current = null;
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  return (
    <div className="container">
      <h1>MP3 Controller</h1>
      <div
        className="upload-section"
        style={{ display: files.length === 0 ? "initial" : "none" }}
      >
        <input
          type="file"
          accept=".mp3,.wav"
          multiple
          onChange={handleFileChange}
          ref={fileInputRef}
          id="file-upload"
        />
        <label htmlFor="file-upload" className="custom-upload">
          Choose MP3 Files
        </label>
      </div>
      <div
        className="player-section"
        style={{
          marginBottom: "20px",
          padding: "15px",
          background: "#f4f4f4",
          borderRadius: "8px",
          display: files.length === 0 ? "none" : "initial",
        }}
      >
        <h3
          style={{
            marginTop: 0,
          }}
        >
          Global Preview
        </h3>
        {/* Visible Main Player */}
        <audio ref={audioRef} controls style={{ width: "100%" }} />

        {/* Hidden Loader (Invisible to user) */}
        <audio
          ref={loaderRef}
          style={{ display: "none" }}
          preload="auto"
          muted
        />
      </div>

      {files.length === 0 ? null : (
        <div className="list-container">
          <ul className="file-list">
            {files.map((file) => (
              <li key={file.id} className="file-item">
                <div className="file-info">
                  <span className="file-name">{file.name}</span>
                  <span className="file-duration">{file.duration}</span>
                </div>
                <div className="slider-container">
                  <input
                    type="range"
                    min="0"
                    max="100"
                    value={file.volume}
                    onChange={(e) =>
                      updateVolume(file.id, parseInt(e.target.value))
                    }
                  />
                  <span className="volume-label">{file.volume}%</span>
                </div>
              </li>
            ))}
          </ul>
          <button type="button" className="discard-btn" onClick={reset}>
            Discard and Go Back
          </button>
        </div>
      )}
    </div>
  );
}

export default App;
