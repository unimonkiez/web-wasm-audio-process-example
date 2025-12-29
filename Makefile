start-example:
	wasm-pack build --target web && (cd example && deno install && deno run dev)
