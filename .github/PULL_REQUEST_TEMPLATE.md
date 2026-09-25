## What and why / 무엇을, 왜

<!-- Link the issue if there is one. / 관련 이슈가 있으면 연결해 주세요. -->

## How it was checked / 확인한 방법

- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cd app && npm run typecheck && npm test`
- [ ] `python scripts/i18n_check.py` (new UI text has an English translation)
- [ ] E2E on Windows (`cd app && npm run e2e`), if the change touches the UI flow
