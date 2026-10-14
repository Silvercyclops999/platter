# Contributing

Thanks for wanting to help.

## Before you start

For anything bigger than a bug fix, please open an issue or a Discussion first
so we can agree on how it should work before you write the code.

Don't contribute code, CSS, strings, icons or other assets from Spotify,
Plexamp or any other app, not even "just for reference". If you want Platter
to behave like another app, describe the behavior in your own words.

Never commit a Plex token, a server address, a `.env` file or screenshots of
your own library. Screenshots should come from the built-in demo library. The
repo has a pre-commit hook that catches most of this; turn it on with
`git config core.hooksPath scripts/git-hooks`.

## Building and testing

```
npm install
npm run tauri dev          # run the app
npm test                   # UI tests (Vitest)
npm run dev                # UI only, in a browser, against the demo backend
cd src-tauri
cargo clippy --all-targets -- -D warnings
cargo test
```

CI also checks the `transcribe` feature (lyrics transcription with
whisper.cpp), which needs CMake and, on Windows, LLVM. See the README.

To test against a real server while developing, copy `.env.example` to `.env`
and add your token. That's only for development; the app itself signs in
through Plex.

## Pull requests

Keep them small and focused, and include tests. If you add a failure state
(server down, bad token, missing file and so on), add a test that fakes it.
CI runs lint and tests and has to pass. For UI changes, include before and
after screenshots from the demo library.
