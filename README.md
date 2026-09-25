# Trace

A local-first tracing camera built with Rust, Yew, and WebAssembly.

## Run locally

Install the WebAssembly target and Trunk once:

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
```

Then start the app:

```sh
trunk serve
```

Open <http://127.0.0.1:8080>. Camera access works on localhost. To test on an
iPhone, deploy the generated `dist/` directory to an HTTPS host; an iPhone
cannot use camera access from an insecure LAN URL.

## Deploy

Every push to `main` runs the included GitHub Pages workflow. In the repository
settings, choose **Pages → Source → GitHub Actions** once. The deployed app will
be available at <https://mandlcho.github.io/camera-trace/>.

## Included

- Front/rear browser camera access (rear preferred)
- Reference-photo upload and browser-side compression
- Drag, opacity, scale, rotation, flip, and reset controls
- Local IndexedDB history with up to 12 recent projects shown
- Mobile-first layout and iPhone safe-area handling
