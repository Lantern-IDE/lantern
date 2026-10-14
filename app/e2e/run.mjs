// Lantern 실제 앱 E2E (Windows, WebView2).
// 빌드한 앱을 격리된 폴더로 띄우고(사용자 설정·데이터를 건드리지 않음), 모의 모델 서버와 임시 git 프로젝트로
// 핵심 흐름을 자동으로 조작해 확인한다.
//
// 사용:  cd app && npx tauri build --no-bundle && npm run e2e
//        npm run e2e -- --only 격리      (이름에 "격리"가 든 시나리오만)
//        npm run e2e -- --keep           (끝나도 임시 폴더를 남김)
//        npm run e2e -- --trace          (지도 그리기 단계를 콘솔로 받아, 멈추면 마지막 기록을 보여 줌)
//        npm run e2e -- --only "격리|바꾸기"  (여러 시나리오는 |로)
// 실패하면 e2e/results/<시나리오>.png 에 화면을 남긴다.

import { runAll } from "./harness.mjs";
import { mockServer } from "./mock-model.mjs";

// 시나리오는 앞 시나리오가 남긴 상태(파일·작업·설정)를 이어 쓰므로 이 순서대로 돈다
import "./scenarios/01-basics.mjs";
import "./scenarios/02-scm-acp.mjs";
import "./scenarios/03-chat-context.mjs";
import "./scenarios/04-agent-tools.mjs";
import "./scenarios/05-review-security.mjs";
import "./scenarios/06-editor-lifecycle.mjs";

await runAll(() => mockServer.close());
