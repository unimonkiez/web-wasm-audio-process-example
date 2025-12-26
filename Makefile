start:
	(cd web && deno bundle --platform=browser src/index.html --outdir dist/ && cd dist && deno run -ENR jsr:@std/http/file-server)
