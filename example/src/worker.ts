import * as Comlink from "comlink";
import * as wasm from "wasm";
export type { AudioCombiner, SingleAudioFile, SingleAudioFileType } from "wasm";

const api = {
    new: (...args: Parameters<typeof wasm.AudioCombiner.new>) =>
        wasm.AudioCombiner.new(...args),
    new_file: (...args: Parameters<typeof wasm.SingleAudioFile.new>) =>
        wasm.SingleAudioFile.new(...args),
    combine: (
        self: wasm.AudioCombiner,
        ...args: Parameters<typeof wasm.AudioCombiner.prototype.combine>
    ) => self.combine(...args),
    createCombiner: async (
        fileData: Array<{ bytes: Uint8Array; type: wasm.SingleAudioFileType }>,
    ) => {
        const wasmFiles = fileData.map((f) =>
            wasm.SingleAudioFile.new(f.bytes, f.type)
        );

        const combiner = await wasm.AudioCombiner.new(wasmFiles);

        // Wrap the combiner in a proxy so we can call its methods
        return Comlink.proxy({
            async combine(volumes: Uint8Array) {
                const resultFile = await combiner.combine(volumes);
                if (!("bytes" in resultFile)) {
                    throw resultFile;
                }
                // Return just the bytes. This is a standard JS Uint8Array
                // which Comlink can pass back perfectly.
                return {
                    bytes: resultFile.bytes,
                    type: resultFile.type,
                };
            },
            // Good practice: allow manual memory cleanup
            free() {
                combiner.free();
            },
        });
    },
    add: (a: number, b: number) => a + b,
};

type MakeAsync<T> = {
    [K in keyof T]: T[K] extends (...args: infer A) => infer R
        ? (...args: A) => Promise<R>
        : T[K];
};

export type Api = MakeAsync<typeof api>;

Comlink.expose(api);
