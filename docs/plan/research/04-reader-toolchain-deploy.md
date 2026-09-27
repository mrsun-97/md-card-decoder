# Reader report: toolchain_deploy

## Summary
I read the ygo-small-world deploy setup, checked this machine's toolchain, and verified current (2026-09-25) Cloudflare and Rust/WASM facts against official docs, crates.io/npm/GitHub APIs and the wrangler 4.139.0 bundle already in the npx cache. ygo-small-world is a Worker that only serves static files. There is no `main`: `assets.directory` is `./public`, `not_found_handling` is `404-page`, `html_handling` is `auto-trailing-slash`, `_headers` caches `/pics/*` for 30 days, and the config comments and npm `dev`/`deploy` scripts are in Chinese. Recommendation: a Rust workspace (core, wasm, cli) built with plain `cargo`, then `wasm-bindgen-cli` (`--target web`) at a pinned version, then optionally `wasm-opt`, all driven by an npm/Node build script. Skip wasm-pack and trunk. Keep the vanilla JS UI and run the engine in a pool of ordinary Web Workers that share one compiled `WebAssembly.Module`. Do not use WASM threads, rayon or COOP/COEP headers, because threads need nightly Rust and cross-origin isolation, and that breaks both `file://` and cross-origin card images. Use one build output that works both online and when index.html is opened via `file://` (wasm fetched online, embedded as base64 offline), and deploy it exactly like the sibling. Two details matter most: the local `wasm-bindgen` CLI version must match the crate exactly (Arch ships CLI 0.2.128, crates.io is at 0.2.129 as of today), and wrangler ignores `.gitignore`. So `dist/card-images*` must never be inside the directory that gets deployed.

## Findings
- **Sibling project = Worker that only serves static files (no Worker code)**: ygo-small-world/wrangler.jsonc: $schema node_modules/wrangler/config-schema.json; name 'ygo-small-world' (= workers.dev subdomain); compatibility_date '2026-09-24'; NO 'main' (comment explains: Cloudflare serves public/ from the edge, requests to static files aren't billed as Worker usage; add main + assets.binding later if needed); assets {directory './public', not_found_handling '404-page' (needs public/404.html), html_handling 'auto-trailing-slash'}; observability.logs {enabled, invocation_logs, persist: true}. package.json: private, Chinese description, scripts {dev: 'wrangler dev', deploy: 'wrangler deploy'}, devDependencies wrangler ^4.139.0. .gitignore adds node_modules/ and .wrangler/. public/_headers: Chinese comment block + '/pics/*  Cache-Control: public, max-age=2592000'. public/404.html is a zh-CN styled page with a link back to '/'. The README has no dedicated deploy section, only notes on self-hosting images, relative paths that work both locally and online, and the _headers caching rationale. [/home/parry/gitRepos/ygo-small-world/wrangler.jsonc, package.json, .gitignore, public/_headers, public/404.html, README.md:26-44]
- **Local Rust is managed by rustup; the wasm32 target is missing but can be added without sudo**: The Arch package 'rustup 1.29.1-1' owns /usr/bin/rustc, which is a symlink to /usr/bin/rustup (/usr/sbin is a symlink to bin). Toolchains live in ~/.rustup: stable (1.98.1, default) and 1.90.0. Only x86_64-unknown-linux-gnu is installed, and wasm32-unknown-unknown is available. So `rustup target add wasm32-unknown-unknown` works as the normal user. ~/.cargo exists (registry, git) but ~/.cargo/bin does not and is not on PATH, so anything from `cargo install` would not be found without a PATH change or `--root .tools`. None of wasm-pack, wasm-bindgen, wasm-opt, wrangler, trunk, cargo-binstall or wasm-tools is installed. Pacman offers wasm-bindgen 0.2.128, binaryen 130, wasm-pack 0.15.0 and wabt 1.0.41. The machine has 20 cores and 15 GB RAM. [`rustup show`; `pacman -Qo /usr/bin/rustc` -> rustup 1.29.1-1; `pacman -Si wasm-bindgen binaryen wasm-pack`]
- **npm can install wrangler into the project (no global install needed)**: Node is v26.9.0 and npm 12.0.2. The npm registry is reachable, and the latest wrangler is 4.141.0 (engines node>=22). The global npm prefix is /usr, so `-g` would need sudo; install it as a devDependency like the sibling does. The npx cache already holds wrangler 4.139.0. No ~/.config/.wrangler login state was found, so the first deploy needs `npx wrangler login` or a CLOUDFLARE_API_TOKEN. [`npm view wrangler version` -> 4.141.0; ~/.npm/_npx/32026684e21afda6/node_modules/wrangler (4.139.0)]
- **Wrangler serves .wasm as application/wasm, so instantiateStreaming works**: When uploading, wrangler sets each file's content type from its extension using its bundled mime table. That table maps 'application/wasm' to ['wasm'], and any text/* type gets '; charset=utf-8' appended. So `WebAssembly.compileStreaming`/`instantiateStreaming` works on Workers static assets with no `_headers` override. [wrangler-dist/cli.js:161866 getContentType = mime.getType(absFilePath) (+charset for text/*); cli.js contains `"application/wasm": ["wasm"]`; docs: https://developers.cloudflare.com/workers/static-assets/headers/]
- **Wrangler does NOT respect .gitignore: local card-image folders would be uploaded**: createAssetsIgnoreFunction skips only /.assetsignore, /_redirects and /_headers, plus patterns listed in `.assetsignore` (same syntax as .gitignore). The repo's .gitignore excludes dist/card-images/, dist/card-images-high/ and dist/card-images-zh/. Those folders hold 9,125 MD passcodes each, so one or two sets is about 9–18k files, plus the 20 wallpapers (22.7 MB). Deploying with directory ./dist would upload all of it and could reach the 20,000-file Free cap. Deploy from a separate, whitelisted build output (public/), or add an .assetsignore. [wrangler-dist/cli.js:161833-161852; cli.js:181351 MAX_ASSET_COUNT_DEFAULT = 2e4; cli.js:160212 MAX_ASSET_SIZE = 25 MiB; node count of CARD_DATA ids with wm>0 = 9125]
- **Cloudflare static-asset limits and behaviour (verified)**: 20,000 files per Worker version on Free and 100,000 on Paid; 25 MiB per file; no documented total-size cap. Requests for static files are free and unlimited on both plans. Default response headers are 'Cache-Control: public, max-age=0, must-revalidate' plus an ETag (a hash of the file). `_headers` and `_redirects` are supported natively; `_headers` allows up to 100 rules and 2,000 characters per line, supports splats and placeholders, and '! Header' removes a header. The `_headers` file itself is not served. html_handling can be auto-trailing-slash (default), force-trailing-slash, drop-trailing-slash or none. run_worker_first accepts a boolean or up to 100 patterns. Pages docs now say: 'Workers supports most Pages use cases… Start new projects with Workers.' [https://developers.cloudflare.com/workers/platform/limits/ ; https://developers.cloudflare.com/workers/static-assets/billing-and-limitations/ ; https://developers.cloudflare.com/workers/static-assets/headers/ ; https://developers.cloudflare.com/workers/static-assets/routing/advanced/html-handling/ ; https://developers.cloudflare.com/pages/]
- **Compression: the edge compresses wasm/js/json automatically, not images; you cannot serve pre-compressed files without a Worker**: Cloudflare's compressible-types list includes application/wasm, application/javascript, text/javascript and application/json. Images other than svg/ico are excluded. The encoding (zstd, brotli or gzip) depends on the client's Accept-Encoding and the plan, and the default brotli level is about 4. Serving pre-compressed .br files would need Worker code, so it is not worth it here. Measured today: cards-data.js is 2032 KB raw, 542 KB gzip-9, 527 KB br-4 and 390 KB br-11; app.js is 101 KB raw and about 27 KB compressed. Most savings will come from trimming the data format (names/aliases), not from the compression settings. [https://developers.cloudflare.com/speed/optimization/content/compression/ ; node zlib measurement over dist/*.js]
- **The rustwasm org was sunset in 2025; wasm-bindgen and wasm-pack now live under github.com/wasm-bindgen and are maintained**: The Rust project announced on 2025-07-21 that the rustwasm org would be archived (September 2025) and that wasm-bindgen would move to a neutral new org. wasm-pack also ended up there: drager/wasm-pack says 'moved to wasm-bindgen/wasm-pack'. crates.io shows wasm-pack 0.15.0 (2026-05-15) and 0.14.0 (2026-01-20). wasm-bindgen is actively released: 0.2.127 (2026-08-08), 0.2.128 (2026-09-04), 0.2.129 (2026-09-25, library MSRV 1.81, CLI MSRV 1.86). js-sys is at 0.3.106 and wasm-bindgen-test at 0.3.79. The CLI version must equal the wasm-bindgen crate version in Cargo.lock exactly; otherwise it errors with a schema mismatch. [https://blog.rust-lang.org/inside-rust/2025/07/21/sunsetting-the-rustwasm-github-org ; https://github.com/wasm-bindgen/wasm-pack/releases ; https://github.com/wasm-bindgen/wasm-bindgen/releases ; crates.io API queried 2026-09-25]
- **The rustc 1.87+ wasm32 feature defaults make bare `wasm-opt` calls fail**: wasm32-unknown-unknown now enables multivalue, mutable-globals, reference-types, sign-ext, nontrapping-fptoint and bulk-memory by default; simd128 is not on by default. Calling `wasm-opt -O` without feature flags fails with 'Bulk memory operations require bulk memory'. Pass --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue, or leave wasm-opt out of the build. [https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html ; https://github.com/rust-lang/rust/issues/141080]
- **WASM threads need nightly Rust and COOP/COEP headers; a pool of Web Workers is the better fit**: wasm-bindgen-rayon 1.3.0 (2024-12) requires a nightly toolchain (README tests nightly-2025-11-15), rust-src, `-C target-feature=+atomics,+bulk-memory`, `-Z build-std=panic_abort,std`, `--target web` only, and cross-origin isolation (COOP same-origin plus COEP require-corp or credentialless). The atomics feature is still unstable on wasm32-unknown-unknown in 2026. Cross-origin isolation is impossible on file://, and COEP require-corp would block cross-origin card images without CORP headers. The simulation is embarrassingly parallel, so N dedicated Workers, each with its own instance built from one shared compiled WebAssembly.Module (a Module can be posted to workers), give the same speedup on stable Rust with no special headers. [https://github.com/RReverser/wasm-bindgen-rayon ; https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/Module ; https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer]
- **file://: module scripts, module workers and fetch are blocked; classic scripts and Blob-URL workers still work**: Chrome refuses ES-module scripts and fetch() under file:// (origin null, CORS-only schemes); see the WHATWG issue #8121 discussion. The current tool already works around this for its Worker by embedding the worker source as a JS string (simulation-worker-source.js) and starting it from a Blob URL (app.js:1687-1688). The wasm bytes can be handled the same way: a classic script sets a base64 string, the page decodes it, calls WebAssembly.compile(bytes), and posts the Module to Blob workers, which call initSync({ module }). [https://github.com/whatwg/html/issues/8121 ; dist/app.js:1687-1688 ; dist/index.html:134 (classic script tags)]
- **Current JS engine traits that shape the Rust API**: The worker currently receives the whole cards array, including names, via structured clone. It uses Math.random (not seedable, so runs can't be reproduced), sends performance.now-throttled progress every 250 ms, and supports a 'cancel' message. In Rust: pass compact typed arrays; use a seeded PRNG with per-episode seeds so results don't depend on worker count; run work in chunked batches so the worker can see cancel messages; use Worker.terminate() plus a respawn (cheap with a cached Module) as the hard cancel. Also avoid std::time::Instant on wasm32-unknown-unknown (it panics); let JS supply deadlines or use web-time. [dist/simulation-worker.js:17,29,31,33,37 ; grep counts: Math.random 9x app.js, 3x worker; performance.now 3x worker]
- **Workers Builds (git integration) has no Rust; GitHub Actions + wrangler-action@v4 is the CI path**: The Workers Builds image preinstalls Node 24.18.0, Python 3.13.3, Go 1.24.3 and Ruby 3.4.4, but not Rust. Free tier: 3,000 build minutes/month, 1 concurrent build, 20-minute timeout. Cloudflare's GitHub Actions guide uses cloudflare/wrangler-action@v4 with the secrets CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID and the 'Edit Cloudflare Workers' token template. Latest action versions: actions/checkout v7.0.1, actions/setup-node v7.0.0, upload-artifact v7.0.1, download-artifact v8.0.1, Swatinem/rust-cache v2.9.2, taiki-e/install-action v2.87.20 (can install wasm-bindgen@<ver> and wasm-pack), wrangler-action v4.1.3. dtolnay/rust-toolchain@master accepts `targets: wasm32-unknown-unknown`. [https://developers.cloudflare.com/workers/ci-cd/builds/build-image/ ; https://developers.cloudflare.com/workers/ci-cd/builds/limits-and-pricing/ ; https://developers.cloudflare.com/workers/ci-cd/external-cicd/github-actions/ ; GitHub releases API 2026-09-25]
- **Card images online: static files for one merged set; R2 as phase 2**: Images are displayed at up to 292 CSS px wide (.recommend-image) and can be zoomed, so ~420x614 WebP is enough. Uploading both sets (high-res en jpg plus zh webp) would be about 18.3k files, too close to the 20k Free cap as the pool grows. One merged set (zh when available, otherwise en) of resized WebP is about 9.1k files and fits. R2 free tier: 10 GB-month, 1M Class A and 10M Class B operations, free egress. r2.dev URLs are rate-limited, meant for development only, and not cached. A custom domain needs a Cloudflare zone. Serving R2 through the same Worker with run_worker_first ['/card-images/*'] counts against 100k requests/day on Free. Hotlinking YGOPRODeck conflicts with the user's own stated policy in the sibling README. [dist/styles.css:192,288 ; https://developers.cloudflare.com/r2/pricing/ ; https://developers.cloudflare.com/r2/buckets/public-buckets/ ; https://developers.cloudflare.com/workers/platform/pricing/ ; ygo-small-world/README.md:26-31]
- **The anaconda curl on PATH has a broken CA bundle**: `curl` resolves to /opt/anaconda/bin/curl, which fails with error 77 (it looks for a missing cacert.pem under ~/.cache/paru/...). Use /usr/bin/curl or `--cacert /etc/ssl/certs/ca-certificates.crt` in any install or download script, such as fetching the binaryen or wasm-bindgen release tarballs or the rustup installer. [`which curl` -> /opt/anaconda/bin/curl; curl: (77) error setting certificate file]

## Spec
# Rust→WASM toolchain and Cloudflare deploy recipe (research as of 2026-09-25)

> Scope: research and plan only. Nothing in the repo was modified. Every "run" command below is for the later coding phase.

## 0. Recommendation summary

| Decision | Recommendation | Why |
|---|---|---|
| Engine language | Rust workspace: `decoder-core` (pure, no wasm deps), `decoder-wasm` (cdylib + wasm-bindgen facade), `decoder-cli` (native, rayon) | Core is testable and benchmarkable natively; the CLI runs 资源价值实验 on 20 native threads; wasm is a thin wrapper |
| Rust→WASM build | Plain `cargo build --target wasm32-unknown-unknown`, then `wasm-bindgen --target web` (CLI version pinned to Cargo.lock), then optional `wasm-opt` with explicit feature flags, all driven by `tools/build-wasm.mjs` via npm | Transparent, cross-platform (Node), no wasm-pack magic, no trunk (trunk is for full-Rust UIs) |
| Parallelism | JS WorkerPool of N classic Workers, each with its own wasm instance made from one shared compiled `WebAssembly.Module` | Stable Rust, no COOP/COEP, works on file:// and https; the simulation is embarrassingly parallel |
| Offline | Same build output works on `file://`: base64 wasm plus Blob-URL worker (the current trick, extended) | Keeps the project's "双击 dist/index.html 即可用" promise |
| UI | Keep and refactor the vanilla JS UI (optionally with `// @ts-check` using the wasm-bindgen `.d.ts`); no Leptos/Dioxus/Yew | 104 KB of working Chinese UI; the engine API boundary is small |
| Hosting | Worker that only serves static files, same pattern as ygo-small-world: no `main`, `assets.directory: ./public`, `404-page`, `auto-trailing-slash`, logs on, `_headers` for caching | Static requests are free and unlimited; Cloudflare recommends Workers over Pages for new projects |
| CI/CD | Phase 1: local `npm run deploy` (the sibling's habit). Phase 2: GitHub Actions (fmt/clippy/test, wasm build, artifact) plus `cloudflare/wrangler-action@v4` on main | Workers Builds has no Rust preinstalled |
| Card images online | Phase 1: one merged, resized WebP set (about 9.1k files) as static files, or no images. Phase 2: R2 | 20k-file Free cap; wrangler ignores `.gitignore` |

## 1. How ygo-small-world deploys (the template to copy)

- `wrangler.jsonc` (comments in Chinese explaining each field):
  - `"$schema": "node_modules/wrangler/config-schema.json"` (assumes wrangler installed locally via npm)
  - `"name": "ygo-small-world"`: this is the workers.dev subdomain `<name>.<account-subdomain>.workers.dev`, and the comment warns that renaming changes the URL.
  - `"compatibility_date": "2026-09-24"`
  - **No `main`**: the site is served entirely from `public/`; no Worker code runs, and static requests don't count as Worker usage. The comment says to add `main` + `assets.binding` later if redirects, auth or APIs are needed.
  - `"assets": { "directory": "./public", "not_found_handling": "404-page", "html_handling": "auto-trailing-slash" }` (404-page requires `public/404.html` to exist)
  - `"observability": { "logs": { "enabled": true, "invocation_logs": true, "persist": true } }`
- `package.json`: `private: true`, Chinese `description`, scripts `{ "dev": "wrangler dev", "deploy": "wrangler deploy" }`, `devDependencies: { "wrangler": "^4.139.0" }`.
- `.gitignore`: a `# Node / Cloudflare` block with `node_modules/` and `.wrangler/`.
- `public/_headers`: Chinese comment explaining the rationale, then `/pics/*` → `Cache-Control: public, max-age=2592000` (30 days; file names are passcodes). Pages keep the default revalidation, so a redeploy takes effect immediately.
- `public/404.html`: zh-CN, dark theme, big "404", a friendly line, and a link back to `/`.
- Conventions to keep: Chinese comments inside config files and `_headers`, Chinese README, short npm scripts `dev`/`deploy`, a committed `package.json` plus lockfile, wrangler as a local devDependency, image paths relative to the HTML so the same files work locally and online, and self-hosted images (the README explicitly rejects hotlinking YGOPRODeck).

## 2. Cloudflare Workers static assets: verified facts

| Item | Fact | Source |
|---|---|---|
| Files per version | Free 20,000 / Paid 100,000 | developers.cloudflare.com/workers/platform/limits/ ; wrangler `MAX_ASSET_COUNT_DEFAULT = 2e4` |
| Max file size | 25 MiB (both plans); no documented total cap | same; wrangler `MAX_ASSET_SIZE = 25*1024*1024` |
| Billing | "Requests to static assets are free and unlimited." `run_worker_first` routes do count as Worker requests (Free: 100k/day, 10 ms CPU) | …/static-assets/billing-and-limitations/ ; …/workers/platform/pricing/ |
| MIME | Set at upload from the file extension; `.wasm` → `application/wasm`, text types get `; charset=utf-8`, so `instantiateStreaming`/`compileStreaming` work | wrangler cli.js:161866 (`mime.getType`), mime table contains `application/wasm: [wasm]` |
| Default caching | `Cache-Control: public, max-age=0, must-revalidate` plus `ETag` (file hash) | …/static-assets/headers/ |
| `_headers` | Supported natively in Workers; ≤100 rules, ≤2,000 chars/line, splat `*` / `:placeholder`, `! Name` removes a header, file itself not served | …/static-assets/headers/ ; …/migration-guides/migrate-from-pages/ |
| `_redirects` | Supported natively | migrate-from-pages |
| Ignore file | Only `.assetsignore` (gitignore syntax) plus the built-in `/.assetsignore`, `/_headers`, `/_redirects`. **`.gitignore` is NOT honored.** | wrangler cli.js:161833–161852 ; …/static-assets/binding/ |
| html_handling | `auto-trailing-slash` (default) / `force-trailing-slash` / `drop-trailing-slash` / `none` | …/routing/advanced/html-handling/ |
| Compression | Edge compresses `application/wasm`, JS, JSON, CSS, HTML (zstd, brotli or gzip depending on Accept-Encoding and plan; brotli level ~4). Images (jpg/png/webp) are not compressed. Pre-compressed `.br` would need a Worker script. | …/speed/optimization/content/compression/ |
| Pages vs Workers | Pages docs banner: "Workers supports most Pages use cases… It is Cloudflare's primary platform… Start new projects with Workers." | developers.cloudflare.com/pages/ |
| COOP/COEP | Can be set via `_headers` if ever needed (not recommended here, see §4) | — |
| R2 | Free: 10 GB-month, 1M Class A, 10M Class B, free egress. `r2.dev` is rate-limited, development only, and not cached. A custom domain needs a Cloudflare zone (full or partial CNAME setup) | …/r2/pricing/ ; …/r2/buckets/public-buckets/ |

Measured payload today (for sizing): `cards-data.js` 2032 KB raw, gzip-9 542 KB, br-4 527 KB, br-11 390 KB. `app.js` 101 KB raw, about 27 KB compressed. Wallpapers 22.7 MB total, largest PNG 3.0 MB (all under 25 MiB). Converting wallpapers to WebP/AVIF for the online build is a cheap win.

**Card images:** 9,125 passcodes have wm>0. Two local sets (en jpg plus zh webp) are about 18.3k files, too close to the Free cap. Options:
1. (Phase 1, recommended) The build writes **one merged set** `public/card-images/{id}.webp` (zh if present, else en) resized to about 420 px wide, which covers the 292 CSS px maximum display at 2x plus zoom. That is about 9.1k files, only ever uploaded incrementally (wrangler uploads changed hashes only), cached 30 days via `_headers`. Missing ids return 404, so the existing `onerror` fallback to `card-back.gif` still works (this is also why `404-page` is better than SPA mode).
2. (Phase 2) R2 bucket plus a tiny Worker (`main`, `assets.binding`, `run_worker_first: ["/card-images/*"]`, `r2_buckets` binding) with a long `Cache-Control`. Costs Worker requests (100k/day on Free).
3. Or an R2 custom domain if the user owns a domain on Cloudflare. Not r2.dev (rate-limited, dev only).
Copyright note: KONAMI owns the card art, and 壁纸来源与授权.txt already carries a takedown clause. Keep the same stance online.

## 3. Rust→WASM toolchain status (2026)

- **Org change:** the rustwasm GitHub org was archived (announced 2025-07-21, archived September 2025). **wasm-bindgen** moved to `github.com/wasm-bindgen` with new maintainers and releases roughly monthly: 0.2.127 (2026-08-08), 0.2.128 (2026-09-04), **0.2.129 (2026-09-25)**; library MSRV 1.81, CLI MSRV 1.86. **wasm-pack** now lives at `wasm-bindgen/wasm-pack` too and is maintained: 0.14.0 (2026-01-20), 0.15.0 (2026-05-15; wasm64, `--panic-unwind`, fixed the npm installer 404). Guide URLs: https://wasm-bindgen.github.io/wasm-bindgen/ and https://wasm-bindgen.github.io/wasm-pack/.
- **Version pinning rule:** the `wasm-bindgen` CLI must equal the `wasm-bindgen` crate version in `Cargo.lock` exactly. Pin in `Cargo.toml` with `wasm-bindgen = "=0.2.129"` and have the build script compare `wasm-bindgen --version` against Cargo.lock and fail with a clear message. Note: Arch `pacman -S wasm-bindgen` currently gives **0.2.128**. Either pin the crate to `=0.2.128` or install the CLI project-locally.
- **wasm-pack vs plain pipeline:** wasm-pack is viable again, but its bundled wasm-opt runs without feature flags (it needs `[package.metadata.wasm-pack.profile.release] wasm-opt = [...]`) and it builds npm-package layouts we don't need. For one crate feeding a hand-written loader, three explicit steps are clearer. **trunk** (0.21.14 stable; 0.22 in beta with MSRV 1.96.1) only makes sense for full-Rust UIs. **wasm-server-runner** (1.0.1) is for `cargo run` of wasm binaries (Bevy-style); not relevant.
- **wasm-bindgen target:** `--target web` (ES module glue exporting `default init` and `initSync`). Add `--omit-default-module-path`, which drops the `new URL('…_bg.wasm', import.meta.url)` fallback because we always pass a compiled Module. That makes the glue safe to bundle as IIFE with esbuild for classic/Blob workers. `--target no-modules` still exists and emits a classic global, but deprecating it has been proposed since 2023 (issue #3355). Acceptable only as a fallback if we want zero JS tooling. `bundler` is for webpack/Vite only. New `module` target (source-phase imports) is too new.
- **rustc wasm defaults:** since 1.87 (LLVM 20) wasm32-unknown-unknown enables multivalue, mutable-globals, reference-types, sign-ext, nontrapping-fptoint and bulk-memory; **simd128 is off by default**. Consider `-C target-feature=+simd128` later only if benchmarks show a gain for bitset AND/popcount over ~129 u64 words; all evergreen browsers support Wasm SIMD. **wasm-opt** must be called with `--enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue`, otherwise it fails with "Bulk memory operations require bulk memory" (rust-lang/rust#141080). wasm-opt sources: pacman `binaryen` 130, GitHub release `version_133` (2026-09-21), or npm `binaryen` 132 (a JS build: slower but works on Windows). Make wasm-opt optional in the build script.
- **Data marshalling:** hot, bulk data (the card table columns, candidate bitsets, simulation result histograms) goes as typed arrays (`&[u8]`, `&[i16]`, `&[u32]` in, `Vec<f64>`/`Box<[u32]>` out, which become copied typed arrays) that JS can transfer. Small structured requests and responses (puzzle state, recommendation list with reasons, config) go through **serde-wasm-bindgen 0.6.5** (smaller and faster than JSON, `serialize_maps_as_objects(true)` for plain objects), or `serde_json` strings if debug-friendliness matters more. Calls are per user action or per batch, so marshalling is negligible.
- **Randomness:** avoid getrandom in the engine. Seed a hand-rolled SplitMix64/xoshiro256++ (or `rand_xoshiro` 0.8.1 without default features) from JS `crypto.getRandomValues`. Per-episode seed = `splitmix(base_seed ^ episode_index)`, so results are reproducible and independent of worker count. If getrandom is ever needed: `getrandom = { version = "0.4", features = ["wasm_js"] }` in the **binary/cdylib crate only**; 0.4 no longer needs the `--cfg getrandom_backend` RUSTFLAG.
- **Other wasm32 gotchas:** `std::time::Instant::now()` panics on wasm32-unknown-unknown, so let JS pass deadlines or batch sizes (or use `web-time` in the wasm crate only). `std::collections::HashMap`'s default hasher works but is slow; use `rustc-hash` (FxHashMap) for memo and transposition tables (deterministic and fast). `f64::ln/exp` may differ in the last ulp between native and wasm, so JS↔Rust parity tests should compare entropy-like values with a tolerance. Use `panic = "abort"` and `console_error_panic_hook` (feature-gated) in the wasm crate.
- **Local install (later, not now):**
  ```sh
  rustup target add wasm32-unknown-unknown          # rustup-managed (Arch 'rustup' pkg); no sudo
  # wasm-bindgen CLI, pick one:
  cargo install wasm-bindgen-cli --version 0.2.129 --locked --root .tools   # exact match; binary in ./.tools/bin (gitignored)
  #   or: sudo pacman -S wasm-bindgen   (0.2.128 → pin crate "=0.2.128")
  #   or: download the prebuilt tarball from github.com/wasm-bindgen/wasm-bindgen/releases with /usr/bin/curl
  sudo pacman -S binaryen                           # optional wasm-opt (130)
  npm install -D wrangler esbuild                   # wrangler 4.141.0 (node>=22 OK), esbuild 0.28.x
  ```
  Note: `~/.cargo/bin` is not on PATH on this box. Use `--root .tools` and have the build script prepend `.tools/bin`. Use `/usr/bin/curl`: the anaconda curl on PATH has a broken CA bundle.

## 4. WASM threads vs a Worker pool

| | wasm-bindgen-rayon (shared-memory threads) | N Workers × own wasm instance (recommended) |
|---|---|---|
| Toolchain | nightly (README: nightly-2025-11-15) + rust-src + `-Z build-std=panic_abort,std` + `+atomics,+bulk-memory`, `--target web` only; atomics still unstable in 2026 | stable 1.98.1 |
| Headers | COOP `same-origin` + COEP `require-corp`/`credentialless` (SharedArrayBuffer) | none |
| file:// | impossible (no cross-origin isolation) | works (Blob workers) |
| Cross-origin images | blocked under require-corp unless the image host sends CORP | unaffected |
| Memory | one shared memory | N copies of the card table (about 100 KB each) plus per-worker caches (cap them) |
| Fit | shared-state parallel search | simulation episodes and slices of candidate guesses are independent, so merge sufficient statistics (sum, sumsq, histograms, best-guess argmax) |

Pool design: `n = clamp(navigator.hardwareConcurrency - 1, 1, 8)`. The main thread compiles once and `postMessage(module)` to each worker (MDN: a Module "can be efficiently shared with Workers"). Each worker calls `initSync({ module })` and builds an `Engine` from transferred typed arrays. Work runs in chunks of about 50–200 ms so the worker can see `cancel` between chunks and send progress (replaces the current 250 ms `performance.now` pulse). Hard cancel = `worker.terminate()` plus respawn from the cached Module. The native `decoder-cli` uses rayon over the same sharding for experiments.

## 5. Offline / file:// story

Facts: under `file://`, Chrome blocks `<script type="module">`, module workers, `new Worker('file:…')` and `fetch()` (origin `null`). Classic `<script src>` tags, Blob-URL workers, `atob`/`WebAssembly.compile(bytes)` and posting a Module all work. The current tool already relies on this (`window.SIMULATION_WORKER_SOURCE` → Blob worker, app.js:1687).

**Recommendation: one build output, two loading paths chosen at runtime:**
- `https:` (Cloudflare, `wrangler dev`, any static server): `WebAssembly.compileStreaming(fetch('engine/decoder_bg.<hash>.wasm'))`; workers from `engine/worker.<hash>.js` (classic script).
- `file:`: inject a classic `<script src="engine/wasm-b64.<hash>.js">` only in this mode (sets `window.ENGINE_WASM_B64`), decode with `Uint8Array.fromBase64` when present or an `atob` fallback, `WebAssembly.compile`. The worker comes from a Blob of `window.ENGINE_WORKER_SOURCE` (the same IIFE bundle embedded as a string, generalizing `tools/build-simulation-worker.mjs`).
- Base64 adds about 33% (about 5–10% after compression) and is only downloaded in file:// mode, so online users don't pay it.
- Card data stays `cards-data.js` (classic `window.CARD_DATA`, still produced by the existing pool tools). At startup, JS packs the numeric columns into typed arrays (b u8, attr-index u8, race-index u8, n u8, nm u16, atk i16, def i16, wm/wa u8; about 90 KB for 8,210 groups) and **transfers** them to workers, instead of structured-cloning 8k objects with names as today. Later, split names/aliases into a lazily loaded file to cut first-paint bytes.
- Secondary option for power users: `npx wrangler dev` (identical `_headers`/404 semantics at http://localhost:8787, reachable from the Windows browser under WSL2) or `python -m http.server`.
- Distribution for zip users: attach an offline zip of `public/` (minus card images) to GitHub Releases from CI, or keep committing build artifacts (see open questions).

## 6. Frontend options for this project

| Option | Pros | Cons for THIS repo |
|---|---|---|
| **A. Keep vanilla JS, refactor (recommended)** | 104 KB Chinese UI and 37 KB CSS already work; DOM/image-heavy features (onerror fallback chains, zoom, wallpapers, themes) are trivial in JS; classic scripts keep file:// working; fast edit-refresh; wasm-bindgen `.d.ts` enables `// @ts-check` + `tsc --noEmit` without converting to TS | app.js needs splitting into classic-script modules (or an esbuild IIFE bundle); two languages |
| B. TypeScript + Vite/esbuild | Types, HMR | Vite emits ES modules and fetches wasm, so file:// breaks unless using a single-file plugin plus inlined wasm; more tooling for a solo project. esbuild alone (IIFE output) is the light version of this |
| C. Full-Rust UI (Leptos 0.8.20 / Dioxus 0.7.10 / Yew 0.23.0) | One language, shared types | Weeks to rewrite the UI; trunk/dx output is ES module plus fetched wasm, so no double-click offline; UI wasm adds hundreds of KB; heavy compute would still need a separate worker wasm (gloo-worker etc.) to avoid freezing the UI; every UI tweak needs a Rust rebuild; small benefit because the engine boundary is only about 10 calls |

The engine API boundary to design (sketch): `Engine::new(columns…)`, `candidates(state) -> bitset/count`, `feedback_partition(state, guess)`, `recommend(state, mode, budget) -> Recommendation`, `simulate_batch(config, seed, first_episode, count) -> Stats`, `hint_value(state)`.

## 7. CI

- **Workers Builds (git integration): not recommended.** Its build image has Node 24.18, Python 3.13, Go 1.24 and Ruby 3.4 but **no Rust**. Installing rustup each build is possible within the 20-minute timeout (Free: 3,000 min/month, 1 concurrent build) but slow and fragile. It only makes sense if the built wasm is committed, so the deploy command is just `npx wrangler deploy`.
- **GitHub Actions (recommended):**
  ```yaml
  name: ci
  on:
    push: { branches: [main] }
    pull_request: {}
  jobs:
    build:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v7
        - uses: dtolnay/rust-toolchain@master
          with: { toolchain: stable, targets: wasm32-unknown-unknown, components: "rustfmt, clippy" }
        - uses: Swatinem/rust-cache@v2
        - uses: taiki-e/install-action@v2
          with: { tool: wasm-bindgen@0.2.129 }        # must equal Cargo.lock
        - uses: actions/setup-node@v7
          with: { node-version: 24, cache: npm }
        - run: npm ci
        - run: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
        - run: cargo test --workspace
        - run: npm run build                           # wasm + site → public/
        - run: node --test tests/                      # JS↔wasm parity / smoke tests
        - uses: actions/upload-artifact@v7
          with: { name: site, path: public }
    deploy:
      needs: build
      if: github.event_name == 'push' && github.ref == 'refs/heads/main'
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v7
        - uses: actions/download-artifact@v8
          with: { name: site, path: public }
        - uses: actions/setup-node@v7
          with: { node-version: 24, cache: npm }
        - run: npm ci                                   # wrangler-action reuses the local wrangler
        - uses: cloudflare/wrangler-action@v4
          with:
            apiToken: ${{ secrets.CLOUDFLARE_API_TOKEN }}   # token from the "Edit Cloudflare Workers" template
            accountId: ${{ secrets.CLOUDFLARE_ACCOUNT_ID }}
            command: deploy
  ```
  Optional: a tag-triggered release job that zips `public/` (without card-images) via `softprops/action-gh-release@v3`. Check the Node version choice (Workers Builds uses 24; local is 26; wrangler needs 22 or newer).

## 8. Concrete recipe

### 8.1 File layout (proposal)
```text
md-card-decoder/
├─ Cargo.toml                # [workspace] members = ["crates/*"], resolver = "3"; shared [profile.release]
├─ Cargo.lock
├─ rust-toolchain.toml       # channel="stable", targets=["wasm32-unknown-unknown"], components=["rustfmt","clippy"], profile="minimal"
├─ crates/
│  ├─ decoder-core/          # data model, bitset index, feedback partition, solvers, simulator, PRNG (no wasm deps)
│  ├─ decoder-wasm/          # crate-type=["cdylib","rlib"]; #[wasm_bindgen] Engine; wasm-bindgen "=0.2.129", js-sys, serde-wasm-bindgen
│  └─ decoder-cli/           # native: experiments, benches (criterion 0.8), rayon, parity checks vs JS reference
├─ web/                      # UI source (today's dist/ minus generated files): index.html, 404.html, _headers, app/*.js, engine-client.js, worker-entry.js, styles.css, cards-data.js, presets.js, wallpapers/
├─ tools/                    # build-wasm.mjs, build-site.mjs (+ existing update-master-duel-pool.mjs, download-card-images.mjs, refine-…)
├─ public/                   # GENERATED (gitignored): the wrangler assets dir AND the offline zip content
├─ .tools/                   # project-local wasm-bindgen/wasm-opt binaries (gitignored)
├─ package.json, package-lock.json, wrangler.jsonc
└─ .github/workflows/ci.yml
```
`build-site.mjs` copies a **whitelist** from `web/` into `public/`, content-hashes JS/CSS/wasm names and rewrites `index.html`, emits `engine/wasm-b64.<hash>.js` and the `ENGINE_WORKER_SOURCE` string, and optionally generates the merged `card-images/` WebP set. Because it never copies `dist/card-images*`, the `.gitignore` gap can't bite. Add a `public/.assetsignore` anyway as a belt-and-braces measure.

### 8.2 Build commands (what `npm run build` does)
```sh
cargo build -p decoder-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --omit-default-module-path --out-dir build/wasm --out-name decoder \
  target/wasm32-unknown-unknown/release/decoder_wasm.wasm          # keep .d.ts for // @ts-check
wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
  --enable-mutable-globals --enable-reference-types --enable-multivalue \
  build/wasm/decoder_bg.wasm -o build/wasm/decoder_bg.wasm          # optional; skip if not installed
npx esbuild web/worker-entry.js --bundle --format=iife --minify --outfile=build/worker.js
node tools/build-site.mjs                                          # → public/
npx wrangler dev      # preview at http://localhost:8787
npx wrangler deploy   # → https://md-card-decoder.<subdomain>.workers.dev
```
Cargo profile: `[profile.release] opt-level = 3, lto = "fat", codegen-units = 1, panic = "abort"` (engine speed matters more than roughly 50 KB of size). Keep native benches in a separate profile if `panic=abort` is unwanted there.

### 8.3 package.json (sibling style)
```json
{
  "name": "md-card-decoder-site",
  "version": "0.0.0",
  "private": true,
  "description": "卡片解码者：Rust/WASM 求解器 + 静态页面，Cloudflare Workers 发 public/",
  "scripts": {
    "build:wasm": "node tools/build-wasm.mjs",
    "build": "node tools/build-wasm.mjs && node tools/build-site.mjs",
    "dev": "npm run build && wrangler dev",
    "deploy": "npm run build && wrangler deploy",
    "test": "cargo test --workspace && node --test tests/"
  },
  "devDependencies": { "wrangler": "^4.141.0", "esbuild": "^0.28.2" }
}
```

### 8.4 wrangler.jsonc (sketch, Chinese comments per convention)
```jsonc
{
  // 装了 node_modules 之后，编辑器按这个 schema 做补全和校验
  "$schema": "node_modules/wrangler/config-schema.json",

  // workers.dev 二级域名：md-card-decoder.<账号子域>.workers.dev；改名 = 换地址
  "name": "md-card-decoder",
  "compatibility_date": "2026-09-25",

  // 没有 "main"：纯静态资源站。求解器是 WASM，跑在访客浏览器的 Web Worker 里，
  // Cloudflare 只负责发文件；静态资源请求免费且不限量。
  "assets": {
    // 构建产物目录（npm run build 生成，不进 git）。注意 wrangler 不看 .gitignore，
    // 所以绝不能直接指向带本地卡图的 dist/。
    "directory": "./public",
    // 找不到的路径回落 public/404.html；缺图时返回 404，卡图 onerror 才能退回卡背
    "not_found_handling": "404-page",
    "html_handling": "auto-trailing-slash"
  },

  "observability": {
    "logs": { "enabled": true, "invocation_logs": true, "persist": true }
  }
}
```
Phase-2 R2 variant (only if needed): add `"main": "worker/index.js"`, `"assets": { …, "binding": "ASSETS", "run_worker_first": ["/card-images/*"] }`, `"r2_buckets": [{ "binding": "CARD_IMAGES", "bucket_name": "md-card-images" }]`.

### 8.5 public/_headers (sketch)
```text
# Cloudflare Workers 静态资源的响应头规则（这个文件本身不会被当成页面发出去）。
# 页面本身（index.html）保持默认：max-age=0, must-revalidate + ETag，重新部署立刻生效。

# 文件名里带内容哈希的构建产物：内容一变文件名就变，可以放心缓存一年
/engine/*
  Cache-Control: public, max-age=31536000, immutable
/assets/*
  Cache-Control: public, max-age=31536000, immutable

# 卡图文件名就是卡片密码，基本不变，缓存 30 天（沿用小世界项目的做法）
/card-images/*
  Cache-Control: public, max-age=2592000
/wallpapers/*
  Cache-Control: public, max-age=2592000

/*
  X-Content-Type-Options: nosniff
  Referrer-Policy: strict-origin-when-cross-origin

# 不需要 COOP/COEP：不用 WASM 多线程 / SharedArrayBuffer。
# 若以后真要上 wasm-bindgen-rayon，才加：
#   Cross-Origin-Opener-Policy: same-origin
#   Cross-Origin-Embedder-Policy: require-corp
# 若要加 CSP，WASM 编译需要 'wasm-unsafe-eval'，Blob Worker 需要 blob:
#   Content-Security-Policy: default-src 'self'; script-src 'self' 'wasm-unsafe-eval' blob:; worker-src 'self' blob:; img-src 'self' data:; style-src 'self' 'unsafe-inline'
```
No `.wasm` MIME override is needed; wrangler already sets `application/wasm`.

### 8.6 Loader sketch (engine-client.js, classic script)
```js
async function loadEngineModule() {
  if (location.protocol === 'file:') {
    await loadClassicScript(ENGINE_FILES.wasmB64);            // sets window.ENGINE_WASM_B64
    const bytes = Uint8Array.fromBase64 ? Uint8Array.fromBase64(window.ENGINE_WASM_B64)
                                        : Uint8Array.from(atob(window.ENGINE_WASM_B64), c => c.charCodeAt(0));
    return WebAssembly.compile(bytes);
  }
  return WebAssembly.compileStreaming(fetch(ENGINE_FILES.wasm));
}
function workerUrl() {
  return location.protocol === 'file:'
    ? URL.createObjectURL(new Blob([window.ENGINE_WORKER_SOURCE], { type: 'text/javascript' }))
    : ENGINE_FILES.worker;
}
// pool: new Worker(workerUrl()) × n → postMessage({ type:'init', module, columns }, [transferables])
// worker-entry.js: import { initSync, Engine } from '../build/wasm/decoder.js';  initSync({ module }); …
```

## 9. Spikes to run first in the coding phase (about 1 hour)
1. Hello-world `decoder-wasm` exporting a `popcount(&[u64])`; build with the pipeline above; load via Blob worker from `file://` in Chrome/Edge and Firefox, checking Module postMessage and `initSync({module})`. Also confirm Safari over https.
2. `npx wrangler dev`: confirm `content-type: application/wasm`, that `_headers` rules apply, and that a missing card image returns 404.
3. Measure the wasm size (raw and br) with and without wasm-opt and `+simd128`.
4. Check that the CLI version equals the crate version in the build script (make a deliberate mismatch to see the error message).

## 10. Sources
- Cloudflare: https://developers.cloudflare.com/workers/platform/limits/ · https://developers.cloudflare.com/workers/static-assets/headers/ · https://developers.cloudflare.com/workers/static-assets/binding/ · https://developers.cloudflare.com/workers/static-assets/routing/advanced/html-handling/ · https://developers.cloudflare.com/workers/static-assets/billing-and-limitations/ · https://developers.cloudflare.com/workers/static-assets/migration-guides/migrate-from-pages/ · https://developers.cloudflare.com/pages/ · https://developers.cloudflare.com/speed/optimization/content/compression/ · https://developers.cloudflare.com/r2/pricing/ · https://developers.cloudflare.com/r2/buckets/public-buckets/ · https://developers.cloudflare.com/workers/platform/pricing/ · https://developers.cloudflare.com/workers/ci-cd/external-cicd/github-actions/ · https://developers.cloudflare.com/workers/ci-cd/builds/build-image/ · https://developers.cloudflare.com/workers/ci-cd/builds/limits-and-pricing/ · https://github.com/cloudflare/wrangler-action
- Rust/WASM: https://blog.rust-lang.org/inside-rust/2025/07/21/sunsetting-the-rustwasm-github-org · https://github.com/wasm-bindgen/wasm-bindgen/releases · https://github.com/wasm-bindgen/wasm-pack/releases · https://wasm-bindgen.github.io/wasm-bindgen/reference/deployment.html · https://wasm-bindgen.github.io/wasm-bindgen/reference/cli.html · https://github.com/wasm-bindgen/wasm-bindgen/issues/3355 · https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html · https://github.com/rust-lang/rust/issues/141080 · https://docs.rs/getrandom/latest/getrandom/ · https://github.com/RReverser/serde-wasm-bindgen · https://github.com/RReverser/wasm-bindgen-rayon · https://blog.rust-lang.org/2025/03/02/Rustup-1.28.0/ · https://github.com/dtolnay/rust-toolchain · https://github.com/taiki-e/install-action (TOOLS.md lists wasm-bindgen, wasm-pack)
- Web platform: https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/Module · https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer · https://github.com/whatwg/html/issues/8121
- Versions from crates.io, npm and GitHub APIs on 2026-09-25; local evidence from the wrangler 4.139.0 bundle at ~/.npm/_npx/32026684e21afda6/node_modules/wrangler/wrangler-dist/cli.js (lines 160212, 161833–161866, 181351).

## Risks
- The wasm-bindgen CLI and crate versions can drift: Arch pacman ships 0.2.128 while crates.io released 0.2.129 today, and wasm-bindgen releases about monthly. Pin with '=' in Cargo.toml and make the build script refuse a mismatch.
- Wrangler ignores .gitignore. Pointing assets.directory at ./dist on a machine with the local card-image folders would upload ~9–18k images and could hit the 20,000-file Free limit or publish hi-res KONAMI art unintentionally.
- The 20,000-files-per-version Free cap will be hit if both image sets go online or the pool grows. Online images need to be one merged set, or R2.
- WebAssembly.Module postMessage into Blob-URL workers from a file:// (opaque-origin) page is expected to work but is unverified on each browser (Chrome/Edge/Firefox/Safari). Spike it before committing to that path; the fallback is posting raw bytes and compiling in each worker.
- A synchronous wasm call blocks its worker's event loop, so 'cancel' messages are only seen between chunks. The engine API must be chunked (simulate_batch) or rely on worker.terminate().
- std::time::Instant panics on wasm32-unknown-unknown. Any time-budgeted search ported from JS (performance.now) must take deadlines or step budgets from JS.
- Native and wasm f64 transcendental results (ln/exp in entropy) may differ in the last ulp. Bit-exact parity tests will be flaky; use tolerances or the libm crate everywhere.
- If build artifacts are not committed, the README's 'download source and open dist/index.html' flow breaks for Windows users without Rust. A CI-built release zip (or committed artifacts) is required.
- Reproducible wasm builds across machines are not guaranteed (absolute cargo registry paths in panic strings). A 'committed artifacts are fresh' CI diff check would need --remap-path-prefix, or should be avoided.
- No wrangler login state was found on this machine (~/.config/.wrangler absent). The first deploy needs `npx wrangler login` (OAuth callback on localhost under WSL2) or an API token.
- The anaconda curl first on PATH has a broken CA bundle. Install scripts using `curl` (rustup, binaryen or wasm-bindgen tarballs) will fail unless /usr/bin/curl or --cacert is used.
- Copyright: card art and several wallpapers are KONAMI or fan works. Putting them on a public workers.dev site increases exposure compared with a local tool; keep the takedown notice and consider omitting hi-res images online.

## Open questions
- Should build artifacts (wasm, base64 wasm, worker string, public/) be committed so zip-download and double-click users keep working without Rust, or should CI publish an offline zip to GitHub Releases and keep main clean? The recommendation is Releases, but it changes the README's quick-start.
- Directory naming: keep `dist/` as the name, or adopt the sibling's `public/` as the generated deploy directory with `web/` as the UI source (proposed)?
- Which card images, if any, should go online: none at first, one merged resized WebP set (~9.1k files, fits Free), or R2 (needs either a Worker script counting against 100k requests/day or a custom domain on Cloudflare)? Does the user own a domain on Cloudflare?
- Worker name and URL: `md-card-decoder` → md-card-decoder.<account-subdomain>.workers.dev. Is that the desired permanent address?
- Is Free plan enough, or is the user on Workers Paid (100k files/version, 10M requests/month)? This affects the image strategy.
- Should the online version keep the Windows `更新大师决斗卡池.cmd` + Node pool-update tools as the single source of cards-data.js (recommended; the engine receives typed arrays at runtime), or should card data be compiled into the wasm with include_bytes (a rebuild, and Rust, needed per pool update)?
- Pin the Rust toolchain to 1.98.1 in rust-toolchain.toml for reproducibility, or track `stable` with an MSRV in Cargo.toml?
- Should CI deploy on every push to main (wrangler-action@v4 with CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID secrets), or keep manual `npm run deploy` like ygo-small-world? Is the repo on GitHub with Actions enabled?
