# DevClash Desktop

The MIT-licensed desktop app for macOS, Windows and Linux. Built with Tauri 2, Rust, React and TypeScript. It reads supported local AI usage and provider quota APIs; local use requires no DevClash account. Cloud history and rankings are optional hosted features.

## Build From Source

Install Node.js 22.22 or newer, Rust stable, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). macOS needs Xcode Command Line Tools; Windows needs the Visual Studio C++ build tools and WebView2; Linux needs the WebKitGTK, build, tray and keyring dependencies listed by Tauri.

```sh
git clone https://github.com/devclash-ai/devclash.git
cd devclash/apps/desktop
npm ci
npm run desktop:dev
```

For a browser-only preview, use `npm run dev`. Native usage collection requires the desktop app.

```sh
npm test
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
```

Build a local unsigned package without DevClash's private release keys:

```sh
npm run desktop:build -- --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

On macOS, add `--bundles app` to build just the app bundle. On Windows use a Developer PowerShell with the C++ build environment available. Packaging outputs are in `src-tauri/target/release/bundle/`. Official signed installers are in [Releases](https://github.com/devclash-ai/devclash/releases).

The repository contains the desktop implementation, lockfiles, bundled pricing, icons and tests. The hosted backend and website, production deployment infrastructure and private signing keys are not included. Source snapshots can contain changes newer than the latest published installer; a source snapshot is not a claim that a new binary has been released.

## Privacy And Authentication

Provider readers access existing local logs and credential stores to retrieve usage and quotas. OS permission prompts may appear. Provider credentials, prompts, responses and local paths are not uploaded by cloud usage sync. Unsupported or unavailable measurements stay unavailable.

DevClash account sign-in opens the hosted website in your browser. Optional cloud functions use the public DevClash API; this source checkout does not include a self-hosted server. Automated tests use fixtures; they do not require provider credentials.

## Updates And Forks

Official builds check `api.devclash.ai` and verify updates using the public key embedded in `src-tauri/tauri.conf.json`. A public verification key is not a signing secret. Existing update downloads continue through the original distribution repository.

Before distributing a fork, change its application identity, branding, API origins, updater endpoint and public verification key to your own. Keep your signing private key outside Git. Otherwise, a locally built app can offer an official DevClash update that replaces it. Disable automatic updates in settings when testing a local build if you do not want this.

## Licenses

See the root [MIT license](../../LICENSE), [OpenUsage notice](LICENSE.upstream) and [pricing provenance](resources/README.md). Dependency licenses remain applicable. Names and logos identify their respective owners; the software license does not grant trademark rights.
