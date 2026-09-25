# Contributing to Lantern

Thanks for taking a look. Lantern is in early beta and maintained by one person, so small, focused changes are the easiest to review.

한국어로 이슈와 PR을 올려도 됩니다.

## Before you start

- **Bugs:** open an issue with steps to reproduce, your OS, and the model/provider you used. `Help → Report Issue…` collects logs (with API keys masked) that you can paste.
- **Features:** open an issue first so we can agree on the approach before you write code. Lantern deliberately stays small; not every good idea will fit.
- **Security issues:** do not open a public issue. See [SECURITY.md](SECURITY.md).

## Development

```bash
cd app && npm install
npx tauri dev
```

Before sending a pull request, run:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && npm run typecheck && npm test
python scripts/i18n_check.py
```

On Windows, also run the end-to-end suite against a release build: `cd app && npx tauri build --no-bundle && npm run e2e`. It runs the real app in isolated folders and never touches your settings.

## Conventions

- **UI text is written in Korean in the source** and translated through `app/src/locales/en.json`. If you add a user-visible string, add its English translation; `scripts/i18n_check.py` fails otherwise.
- Match the surrounding code: comment density, naming, and error handling. Prefer plain, specific wording in the UI.
- Tests must not read or write real user data. Use the `LANTERN_HOME`, `LANTERN_DATA_DIR`, `LANTERN_INDEX_DIR`, and `LANTERN_WEBVIEW_DATA_DIR` overrides.
- If you add a dependency, regenerate `THIRD_PARTY_NOTICES.md` with `python scripts/gen_notices.py` and make sure it reports no licenses that need review.

## Commit messages

```
TYPE: short summary
```

- `TYPE` is uppercase: `FEAT`, `FIX`, `REFACTOR`, `PERF`, `TEST`, `DOCS`, `DESIGN`, `CHORE`
- One concern per commit: one module, one screen, one endpoint
- Add a body only when the *why* isn't obvious; don't restate what the diff shows
- The maintainer writes summaries in Korean (`FEAT: 코드 지도 추가`); English is fine for contributions

## License of contributions

Lantern is licensed under [Apache-2.0](LICENSE). By submitting a contribution you agree that it is licensed under the same terms (Apache-2.0, section 5). Please add a `Signed-off-by` line to your commits (`git commit -s`) to certify the [Developer Certificate of Origin](https://developercertificate.org/).
