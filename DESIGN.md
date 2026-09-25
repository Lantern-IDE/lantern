---
name: Lantern
description: 내 프로젝트를 이해하는, 가볍고 자유로운 AI IDE. VS Code의 배치와 조작 위에 맥락 엔진이 한 일을 보여주는 워크벤치.
colors:
  bg: "#16171c"
  editor-bg: "#1b1c22"
  elevated: "#202128"
  sunken: "#131419"
  border: "#2a2c35"
  border-strong: "#3a3d49"
  fg: "#d5d8e0"
  fg-strong: "#f0f1f5"
  muted: "#9298a8"
  dim: "#6f7585"
  hover: "rgba(160, 168, 200, 0.08)"
  press: "rgba(160, 168, 200, 0.14)"
  active-sel: "rgba(108, 115, 245, 0.26)"
  inactive-sel: "rgba(160, 168, 200, 0.12)"
  accent: "#5b62e6"
  accent-hover: "#6a71f0"
  accent-fg: "#ffffff"
  accent-soft: "rgba(108, 115, 245, 0.16)"
  focus: "#7c83ff"
  link: "#9aa3ff"
  match: "#8f97ff"
  context: "#2cc6e0"
  context-soft: "rgba(44, 198, 224, 0.12)"
  context-line: "rgba(44, 198, 224, 0.35)"
  input-bg: "#1f2027"
  input-border: "#33353f"
  error: "#f2656a"
  error-soft: "rgba(242, 101, 106, 0.12)"
  warning: "#dcae3a"
  warning-soft: "rgba(220, 174, 58, 0.12)"
  ok: "#45bd86"
  info: "#6aa8f8"
  add-bg: "rgba(69, 189, 134, 0.14)"
  del-bg: "rgba(242, 101, 106, 0.14)"
  search-hit: "rgba(220, 174, 58, 0.30)"
  badge-bg: "#3a3d49"
  badge-fg: "#e6e8ee"
  brand-cyan: "#22d3ee"
  brand-indigo: "#6366f1"
  light-bg: "#f2f3f7"
  light-editor-bg: "#ffffff"
  light-sunken: "#e9ebf0"
  light-border: "#dfe1e8"
  light-border-strong: "#c7cad4"
  light-fg: "#2a2d37"
  light-fg-strong: "#14161c"
  light-muted: "#5d6272"
  light-dim: "#8a8f9e"
  light-accent: "#4f55d8"
  light-accent-hover: "#444ac8"
  light-focus: "#5b62e6"
  light-link: "#4147c4"
  light-context: "#0b7f94"
  light-input-border: "#cfd2db"
  light-error: "#c9353b"
  light-warning: "#9a6b00"
  light-ok: "#1d8a57"
  light-info: "#2a6fd1"
typography:
  display:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI Variable Text, Segoe UI, Malgun Gothic, system-ui, sans-serif"
    fontSize: "30px"
    fontWeight: 700
    letterSpacing: "-0.01em"
  headline:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI Variable Text, Segoe UI, Malgun Gothic, system-ui, sans-serif"
    fontSize: "22px"
    fontWeight: 700
    letterSpacing: "-0.01em"
  title:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI Variable Text, Segoe UI, Malgun Gothic, system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 700
  body:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI Variable Text, Segoe UI, Malgun Gothic, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.45
  body-reading:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI Variable Text, Segoe UI, Malgun Gothic, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.65
  small:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI Variable Text, Segoe UI, Malgun Gothic, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.55
  label:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI Variable Text, Segoe UI, Malgun Gothic, system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 600
    letterSpacing: "0.04em"
  code:
    fontFamily: "Cascadia Code, Cascadia Mono, Consolas, D2Coding, monospace"
    fontSize: "14px"
    lineHeight: 1.45
  code-small:
    fontFamily: "Cascadia Code, Cascadia Mono, Consolas, D2Coding, monospace"
    fontSize: "12px"
    lineHeight: 1.5
rounded:
  sm: "4px"
  md: "6px"
  lg: "8px"
  pill: "10px"
spacing:
  xs: "4px"
  sm: "6px"
  md: "8px"
  lg: "12px"
  xl: "16px"
  inset: "18px"
  2xl: "24px"
  3xl: "36px"
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-fg}"
    rounded: "{rounded.sm}"
    padding: "0 12px"
    height: "28px"
  button-primary-hover:
    backgroundColor: "{colors.accent-hover}"
  button-secondary:
    backgroundColor: "transparent"
    textColor: "{colors.fg}"
    rounded: "{rounded.sm}"
    padding: "0 12px"
    height: "28px"
  button-secondary-hover:
    backgroundColor: "{colors.hover}"
  button-ghost:
    textColor: "{colors.link}"
    rounded: "{rounded.sm}"
    padding: "0 6px"
    height: "28px"
  button-ghost-hover:
    backgroundColor: "{colors.accent-soft}"
  icon-button:
    textColor: "{colors.fg}"
    rounded: "{rounded.sm}"
    size: "24px"
  icon-button-hover:
    backgroundColor: "{colors.hover}"
  send-button:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-fg}"
    rounded: "{rounded.sm}"
    size: "26px"
  send-button-disabled:
    backgroundColor: "{colors.inactive-sel}"
    textColor: "{colors.dim}"
  input:
    backgroundColor: "{colors.input-bg}"
    textColor: "{colors.fg}"
    rounded: "{rounded.sm}"
    padding: "4px 8px"
    height: "28px"
  composer:
    backgroundColor: "{colors.input-bg}"
    rounded: "{rounded.lg}"
  chip-button:
    textColor: "{colors.muted}"
    typography: "{typography.small}"
    rounded: "{rounded.sm}"
    padding: "0 6px"
    height: "22px"
  pill-default:
    backgroundColor: "{colors.accent-soft}"
    textColor: "{colors.link}"
    rounded: "{rounded.pill}"
    padding: "0 7px"
    height: "20px"
  badge:
    backgroundColor: "{colors.badge-bg}"
    textColor: "{colors.badge-fg}"
    rounded: "{rounded.pill}"
    padding: "0 6px"
    height: "16px"
  context-card:
    backgroundColor: "{colors.context-soft}"
    textColor: "{colors.context}"
    rounded: "{rounded.md}"
    padding: "6px 10px"
  tab-active:
    backgroundColor: "{colors.editor-bg}"
    textColor: "{colors.fg-strong}"
    height: "36px"
  statusbar-context:
    textColor: "{colors.context}"
    typography: "{typography.small}"
    height: "24px"
  toast:
    backgroundColor: "{colors.elevated}"
    textColor: "{colors.fg}"
    rounded: "{rounded.lg}"
    padding: "10px 10px 10px 12px"
  list-row:
    textColor: "{colors.fg}"
    height: "22px"
  list-row-selected:
    backgroundColor: "{colors.active-sel}"
---

# Design System: Lantern

## Overview

**Creative North Star: "보이는 맥락의 워크벤치 (The Glass Workbench)"**

Lantern은 VS Code의 배치와 조작을 그대로 쓰는 데스크톱 IDE다. 제목 표시줄, 액티비티 바, 탐색기, 탭과 편집기, 하단 패널, 오른쪽 채팅 보조 사이드바, 상태 표시줄이 VS Code와 같은 자리, 같은 높이, 같은 단축키로 놓인다. 개발자가 옮겨 오는 비용이 없게 하는 것이 먼저이고, 정체성은 세부에서만 드러난다.

그 세부는 세 가지다. 무채색 전체가 남색 쪽으로 살짝 기울어 있어 VS Code의 순수 회색과 구별되고, 색은 딱 두 목소리로 말한다. 남색은 사용자가 누르고 초점을 둔 곳, 청록은 맥락 엔진이 한 일이다. 그리고 육각형 표시가 로고, 봇 아바타, 상태 표시줄, 맥락 카드에 반복되며 AI가 일하는 동안에만 윤곽을 따라 선이 돈다. 화면은 13px 본문의 조밀한 도구 밀도를 지키고, 장식 없이 1px 선과 명도 단계로 면을 나눈다.

피하는 인상은 두 가지로 확정되어 있다. 로고만 바꾼 VS Code 복제품, 그리고 빛나는 테두리와 과한 그라데이션으로 된 게이밍·네온 AI 도구.

**Key Characteristics:**
- VS Code 워크벤치 배치: 36px 제목 표시줄·탭·뷰 제목, 48px 액티비티 바, 22px 목록 행, 24px 상태 표시줄.
- 남색 기운의 무채색 네 단계(sunken, bg, editor-bg, elevated)와 1px 선으로 면을 나눈다.
- 남색 = 사용자 동작·포커스, 청록 = 맥락 엔진. 두 색은 한 요소 안에서 섞지 않는다.
- 청록→남색 브랜드 그라데이션은 육각형 표시에만 쓴다.
- 다크가 기본, 라이트는 같은 역할을 명도 대비에 맞춘 값으로 다시 정의한다.
- 반복 모션은 AI 작업 중 육각형 윤곽 추적 하나.

## Colors

남색 기운의 무채색 바탕 위에 남색과 청록 두 역할색만 올리고, 나머지 색은 상태(오류·경고·성공·정보)를 말할 때만 쓴다.

### Primary
- **동작 남색 (Action Indigo)** (`accent`, 라이트 `light-accent`): 사용자가 누르는 주 동작의 채움색. 채팅 보내기 버튼, 주 버튼(적용, 시작하기, 폴더 열기), 활성 탭 위 2px 선, 액티비티 바 활성 막대, 패널·보조 탭 활성 밑줄, 메뉴 항목 hover 채움, 사용량 미터, 읽지 않은 알림 점, 사이드바 크기 조절선 hover.
- **초점 남색 (Focus Indigo)** (`focus`): 모든 `:focus-visible` 외곽선(1px), 입력창 포커스 테두리, 캐럿, 켜진 토글·세그먼트의 안쪽 테두리. 라이트에서는 `accent` 다크 값과 같은 #5b62e6이다.
- **링크 남색 (Link Periwinkle)** (`link`, `match`): 링크, 고스트 버튼 글자, 파일 링크, 기본 모델 표시(pill), 빠른 열기와 자동 완성의 일치 글자.
- **선택 남색** (`active-sel`, `accent-soft`, `selection`): 포커스가 있는 목록 선택 행, 빠른 열기 선택 행, 설정 탐색 활성 항목, 텍스트 선택.

### Secondary
- **맥락 청록 (Context Cyan)** (`context`, 라이트 `light-context`): 맥락 엔진이 한 일에만 쓴다. 상태 표시줄 왼쪽 "맥락 N · 8.0k/8.0k" 항목, 채팅 답변 위 "Lantern이 본 코드" 카드 제목과 육각형, 인덱싱 완료 안내, 시작 화면의 "Lantern이 하는 일" 아이콘, 인스펙터의 요청 수치, 출력 창의 출처, 설정 파일 아이콘, 페이지 탭(시작하기·설정)의 아이콘, 스트리밍 커서.
- **맥락 청록 면** (`context-soft`, `context-line`): 맥락 카드 바탕(12% 청록)과 테두리·구분선(35% 청록).

### Tertiary
- **브랜드 청록 → 남색** (`brand-cyan` → `brand-indigo`, 135deg): SVG 선형 그라데이션 `#lantern-brand`로만 존재하며 `.hex.brand`의 윤곽선과 가운데 점에만 칠해진다. 면이나 글자, 버튼에 쓰지 않는다.

### Neutral
- **깊은 남흑 (Sunken Ink)** (`sunken`): 명령 센터, 코드 블록, 도구 출력, kbd 바탕. 가장 낮은 면.
- **워크벤치 남흑 (Workbench Ink)** (`bg`): 제목 표시줄, 액티비티 바, 사이드바, 탭 줄, 패널, 상태 표시줄.
- **편집기 남흑 (Editor Ink)** (`editor-bg`): 편집기, 활성 탭, 시작 화면, 설정·시작하기 페이지, 맥락 카드 파일 목록, Diff.
- **떠 있는 면 (Raised Slate)** (`elevated`): 메뉴, 토스트, 알림 센터, 빠른 열기, 채팅 안의 도구·승인·예시 카드, 모델 목록, 선택 카드.
- **선** (`border`, `border-strong`): 모든 면 구분은 1px `border`. 떠 있는 층의 테두리, 보조 버튼 테두리, 트리 들여쓰기 안내선 hover에는 `border-strong`.
- **글자** (`fg-strong` 제목·활성 항목, `fg` 본문, `muted` 보조 설명·비활성 탭, `dim` 자리 표시자·빈 상태 문구·경로).
- **상호작용 막** (`hover` 8%, `press` 14%, `inactive-sel` 12%): 모두 남색 기운 회색 rgba(160, 168, 200)의 투명도 단계. 라이트는 rgba(40, 48, 90).

### 상태 색
- `error`, `warning`, `ok`, `info`와 각각의 `-soft` 면, Diff의 `add-bg`/`del-bg`, 검색 일치 `search-hit`. 승인 카드 테두리는 `warning`, 오류 상자는 `error`, 완료 단계 표시와 연결됨 표시는 `ok`.

### 편집기 문법 색과 파일 아이콘 색
문법 색은 VS Code "Dark+"(다크)와 "Light+"(라이트) 값을 그대로 쓴다(`app/src/theme.ts`). 편집기 바탕·선택·캐럿만 Lantern 토큰을 따른다(다크 캐럿 #9aa3ff, 라이트 캐럿 #4f55d8). 파일 아이콘은 Codicons에 언어별 관례 색을 입힌다(TS #3178c6, Rust #d9844a, 기본 #8a8f9e 등, `app/src/icons.ts`).

### Named Rules
**두 목소리 규칙 (The Two Voices Rule).** 남색은 사용자가 한 일과 초점, 청록은 맥락 엔진이 한 일이다. 새 요소를 칠하기 전에 "누가 한 일인가"를 묻고, 답이 둘 다가 아니면 한 요소에 두 색을 섞지 않는다.

**로고 전용 그라데이션 규칙 (The Gradient Stays On The Mark Rule).** 청록→남색 그라데이션은 육각형 표시에만 칠한다. 버튼, 배경, 테두리, 글자에 그라데이션을 쓰지 않는다.

**남색 기운 무채색 규칙 (The Tinted Neutral Rule).** 회색은 모두 남색 쪽으로 기운 값이다. 순수 회색(#1e1e1e, rgba(255,255,255,…) 등)을 새로 들이지 않는다. 막 색은 rgba(160, 168, 200)(다크)과 rgba(40, 48, 90)(라이트)의 투명도로만 만든다.

## Typography

**UI Font:** Pretendard Variable (Segoe UI Variable Text, Segoe UI, Malgun Gothic, system-ui로 대체). npm `pretendard` 동적 서브셋으로 번들되어 오프라인에서 동작한다.
**Code Font:** Cascadia Code (Cascadia Mono, Consolas, D2Coding, monospace로 대체).
**Icons:** VS Code Codicons 아이콘 글꼴(기본 16px, 버튼 안 14px, 액티비티 바 22px).

**Character:** 한글이 고른 Pretendard가 13px의 조밀한 도구 밀도를 받치고, Cascadia가 코드·심볼·경로·키 입력칸을 맡는다. 굵기는 400·600·700 세 단계만 쓴다.

### Hierarchy
- **Display** (700, 30px, -0.01em): 폴더 없을 때 시작 화면의 "Lantern" 한 곳.
- **Headline** (700, 22px, -0.01em): 설정·시작하기 페이지의 h1.
- **Title** (700, 14–16px): 페이지 안 섹션 제목(14px), 설정 섹션 제목(16px), 빈 채팅 제목(15px), 선택 카드 제목(13.5px).
- **Body** (400, 13px, 1.45): 워크벤치 전체의 기본. 트리, 메뉴, 탭, 입력.
- **Body reading** (400, 13px, 1.6–1.65): 채팅 답변 마크다운과 사용자 메시지, 빈 상태 설명. 읽는 글만 줄간격을 넓힌다.
- **Small** (400, 12–12.5px, 1.55): 도움말, 보조 설명, 상태 표시줄, 카드 안 글자, 브레드크럼(12.5px).
- **Label** (600, 11px, 0.04em): VS Code 뷰 제목(탐색기·검색), 패널 탭(문제·출력·터미널), 채팅/인스펙터 탭, 알림 센터 머리, 메뉴 그룹 머리. 대문자 변환 없이 한글 그대로 쓴다.
- **Code** (14px 편집기, 1.45 / 12–12.5px 채팅·도구·출력, 1.5): 코드, 심볼 이름, 경로, API 키 입력, 태그.

### Named Rules
**표 숫자 규칙 (The Tabular Figures Rule).** 토큰 수, 비용, 줄·열 위치, 배지 숫자, 단계 번호는 항상 `font-variant-numeric: tabular-nums`로 쓴다. 숫자가 바뀌어도 자리가 흔들리지 않아야 한다.

**13px 밀도 규칙 (The 13px Density Rule).** 워크벤치 본문은 13px이다. 크기를 키워 위계를 만드는 것은 페이지(시작 화면, 설정, 시작하기) 제목에만 허용하고, 워크벤치 안에서는 굵기와 `fg-strong`/`muted` 대비로 위계를 만든다.

## Layout

VS Code 워크벤치 격자를 그대로 쓴다. 세로로 제목 표시줄(36px) / 워크벤치 / 상태 표시줄(24px). 워크벤치는 가로로 액티비티 바(48px) / 기본 사이드바(기본 260px, 최대 28vw, 최소 170px) / 편집기 영역(탭 36px, 브레드크럼 24px, 편집기, 하단 패널 기본 240px) / 채팅 보조 사이드바(기본 380px, 최대 36vw, 최소 280px). 사이드바·패널 경계에는 4px 크기 조절선(sash)이 있고, hover나 드래그 중에 0.25초 지연 뒤 남색으로 드러난다.

제목 표시줄은 1fr / auto / 1fr 격자로 가운데에 명령 센터(최대 560px, 38vw)를 둔다. 오른쪽에는 레이아웃 전환 버튼과 46px 폭의 창 조작 버튼이 온다.

목록은 22px 행, 뷰 안쪽 왼쪽 여백은 18px로 통일된다(뷰 제목, 빈 상태, 검색 상자, 출력). 간격은 4·6·8·10·12·14·16px의 작은 단계를 쓰고, 페이지 영역만 24·28·36·48px로 넓어진다. 페이지(설정 최대 820px, 시작하기 최대 820px, 시작 화면 최대 760px)는 편집기 탭 안에서 가운데 정렬된 읽기 폭을 가진다. 설정은 180px 왼쪽 탐색 + 본문 2열이다.

**반응형.** 창 폭 1100px 이하에서 메뉴 막대는 햄버거 하나로 접히고, 두 사이드바는 각각 21vw·28vw로 줄어 편집기 폭을 확보한다. 채팅 입력줄은 컨테이너 폭 300px 이하에서 칩 글자를 숨기고 아이콘만 남긴다. 설정의 모델 동작 버튼은 컨테이너 760px 이하에서 아래 줄로 내려간다. 시작하기 선택 카드는 컨테이너 620px 이하에서 한 열로 쌓이도록 되어 있다. 창 최소 크기는 900×560이다.

## Elevation & Depth

면은 평평하다. 깊이는 무채색 명도 네 단계(sunken < bg < editor-bg < elevated)와 1px 선으로 만든다. 그림자는 화면 위에 떠서 다른 것을 가리는 층에만 있다.

### Shadow Vocabulary
- **떠 있는 층** (`box-shadow: 0 8px 24px rgba(0, 0, 0, 0.42), 0 1px 3px rgba(0, 0, 0, 0.3)`, 라이트 `0 8px 24px rgba(20, 24, 40, 0.14), 0 1px 3px rgba(20, 24, 40, 0.1)`): 메뉴 팝업, 토스트, 알림 센터, 빠른 열기, 편집기 툴팁·자동 완성.
- **안쪽 초점 테두리** (`box-shadow: inset 0 0 0 1px var(--focus)`): 포커스가 있는 트리 선택 행, 켜진 세그먼트 버튼. 그림자가 아니라 테두리 역할이다.

### Named Rules
**떠 있는 것만 그림자 규칙 (The Only-Floaters-Cast Rule).** 워크벤치 면, 카드, 채팅 안 블록에는 그림자를 주지 않는다. 그림자는 겹쳐 떠 있는 층(메뉴, 토스트, 알림, 빠른 열기, 툴팁)의 표시다.

**빛나지 않는 규칙 (The No-Glow Rule).** 색 있는 번짐 그림자나 빛나는 테두리를 쓰지 않는다. 강조는 1px 선 색과 옅은 면 색(`-soft`)으로만 한다.

## Shapes

모서리는 작고 일정하다. 4px(`sm`)는 버튼, 입력, 칩, 태그, 목록 행(빠른 열기·메뉴 항목), 6px(`md`)는 명령 센터, 메뉴 팝업, 채팅 안 카드(맥락·도구·승인·오류·예시), 코드 블록, 세그먼트. 8px(`lg`)는 한 덩어리로 떠 있거나 묶인 큰 용기(채팅 입력창, 토스트, 알림 센터, 빠른 열기, 모델 목록, 선택 카드, 모델 추가 점선 상자). 10px(`pill`)는 배지와 상태 알약. 원은 아바타, 단계 표시, 알림 점, 액티비티 바 표시 점뿐이다.

반복되는 형태는 육각형이다. 16×16 viewBox의 꼭짓점이 위로 향한 정육각형 윤곽(1.6 선, 둥근 이음)과 반지름 2.1의 가운데 점. `brand`는 그라데이션, `plain`은 현재 글자색을 따른다. 크기는 13–17px(상태 표시줄, 제목 표시줄, 카드), 20–22px(봇 아바타), 34–36px(시작 화면, 빈 채팅, 시작하기 제목), 120px(빈 편집기 워터마크, 0.7 선, `border-strong` 색 55% 투명).

활성 표시는 2px 막대다. 활성 탭은 위쪽, 패널·보조 탭은 아래쪽, 액티비티 바는 왼쪽에 남색 2px 막대를 둔다.

## Components

### Buttons
조용하고 작다. VS Code 버튼의 크기에 남색 채움 하나.
- **Shape:** 살짝 둥근 모서리 (4px), 높이 28px, 좌우 12px, 아이콘 14px과 6px 간격.
- **Primary:** 남색 채움(`accent`) 위 흰 글자. 한 화면 영역에 하나(적용, 시작하기, 폴더 열기, 저장하고 확인 이후의 주 동작).
- **Hover / Focus:** hover에서 `accent-hover`로 밝아진다(80ms). 포커스는 전역 1px `focus` 외곽선.
- **Secondary:** 투명 바탕, `border-strong` 1px 테두리, `fg` 글자. hover에서 `hover` 막.
- **Ghost:** 테두리 없이 `link` 글자, hover에서 `accent-soft` 면.
- **Icon button:** 24×24, 4px 모서리, hover `hover`, 눌림 `press`. 뷰 제목의 동작 버튼은 뷰에 마우스를 올리거나 포커스가 들어올 때만 보인다(채팅 사이드바는 항상 보임).
- **Disabled:** 45% 불투명도. 보내기 버튼만 예외로 `inactive-sel` 면과 `dim` 글자로 바뀐다.

### Chips
- **Style:** 채팅 입력줄의 칩(에이전트 선택, 현재 파일, 모델)은 높이 22px, 11.5px `muted` 글자, 투명 테두리. hover에서 `hover` 막과 `fg` 글자.
- **State:** "현재 파일" 칩은 `border` 테두리를 가지며, 꺼지면 50% 불투명도와 취소선. 경고 상태 칩은 `warning` 글자.
- **Pill:** 높이 20px, 10px 모서리, 11px 600 굵기. 기본(`accent-soft` + `link`), 정상(`add-bg` + `ok`), 문제(`error-soft` + `error`), 중립(`inactive-sel` + `muted`).

### Cards / Containers
- **Corner Style:** 채팅 안 블록 6px, 큰 용기 8px.
- **Background:** `elevated`(도구, 승인, 변경 파일, 인스펙터 요청, 모델, 선택 카드). 코드와 출력은 `sunken`, Diff는 `editor-bg`.
- **Shadow Strategy:** 없음. Elevation & Depth 참조.
- **Border:** 1px `border`. 승인 대기 카드는 `warning` 테두리와 `warning-soft` 머리, 결정 후 `border`로 돌아간다. 오류 상자는 `error` 테두리와 `error-soft` 면. 시작하기 선택 카드는 연결되면 `ok` 테두리.
- **Internal Padding:** 머리 6–8px × 10px, 본문 8px × 10px, 모델 행과 선택 카드 14px × 16px.

### Inputs / Fields
- **Style:** `input-bg` 바탕, `input-border` 1px, 4px 모서리, 4px × 8px 안쪽 여백, 목록형 입력은 높이 28px. 자리 표시자는 `dim`. 입력 안 토글 버튼은 22px 정사각형으로 오른쪽에 붙는다.
- **Focus:** 테두리가 `focus` 남색으로 바뀐다(80ms). 캐럿도 `focus`.
- **Toggle on:** `accent-soft` 면 + `focus` 테두리 + `fg-strong` 글자. 세그먼트 버튼도 같은 조합(안쪽 1px 테두리).
- **Error:** 필드 아래 12px `error` 글자. 저장 성공은 `ok` 글자 "저장됨"이 0.2초에 걸쳐 나타난다.

### Navigation
- **액티비티 바:** 48×44 항목, 22px 아이콘, 기본 `dim`, hover `fg`, 활성 `fg-strong` + 왼쪽 2px 남색 막대. 맥락 엔진 알림 점은 7px 청록 원.
- **탭:** 36px, 비활성 `muted` 글자와 `bg` 바탕, 활성은 `editor-bg`로 편집기와 이어지고 위쪽 2px 남색 선. 닫기 버튼은 hover·활성·수정됨일 때만 보이며, 수정된 탭은 9px 채운 원으로 표시한다. 시작하기·설정 같은 페이지 탭의 아이콘은 청록.
- **패널·보조 탭:** Label 서체, 활성 `fg-strong` + 아래 2px 남색 밑줄(좌우 10px 들여씀).
- **메뉴:** `elevated` 팝업, 26px 항목, hover는 남색 채움 + 흰 글자(VS Code 관례), 단축키는 오른쪽 `muted`.
- **상태 표시줄:** 24px, 12px `muted` 글자, 항목 좌우 7px, 클릭 가능 항목만 hover 막. 왼쪽 끝 브랜드 육각형, 그 옆 청록 "맥락" 항목.

### 맥락 카드 (Signature Component)
채팅 답변 위에 붙는 "Lantern이 본 코드" 접이식 카드. 제품의 시그니처 인터랙션이다. 6px 모서리, `context-line` 테두리, `context-soft` 바탕. 요약 줄은 청록 육각형(14px) + 청록 600 굵기 제목 + `muted` 표 숫자 메타(파일 수·심볼, 토큰) + 오른쪽 셰브론(펼치면 90도, 0.15초). 펼치면 `editor-bg` 목록(최대 320px 스크롤)에 파일 행과 들여쓴 심볼 행(코드 서체 이름 + `muted` 이유)이 나오고, 심볼 행을 누르면 해당 위치로 이동한다.

### 채팅 입력창 (Composer)
8px 모서리 `input-bg` 상자에 테두리 없는 텍스트 영역(최소 48px, 최대 220px)과 칩 줄. 상자 안에 포커스가 있으면 테두리가 `focus`로 바뀐다. 오른쪽 끝에 26px 남색 보내기 버튼, 응답 중에는 같은 자리에 `inactive-sel` 중지 버튼.

### 토스트와 알림 센터
오른쪽 아래(열린 채팅 사이드바 폭만큼 비켜서) 최대 420px, `elevated` + `border-strong` + 8px + 떠 있는 층 그림자. 18px 아이콘 열은 종류별 상태 색(info·ok·warning·error), 본문 `fg`, 상세 12px `muted`, 동작은 높이 24px 보조 버튼. 들어올 때 220ms(10px 위로, 0.98 확대), 나갈 때 160ms.

### 육각형 작업 표시 (Motion)
AI가 응답 중이거나 인덱싱 중이면 봇 아바타와 상태 표시줄 맥락 항목의 육각형에 `working`이 붙는다. 윤곽 선이 22/44 대시로 1.4초마다 한 바퀴 돌고(linear), 가운데 점이 같은 주기로 45%까지 흐려진다. `prefers-reduced-motion`에서는 대시를 없애고 모든 애니메이션·전환을 1ms로 줄인다.

그 밖의 모션은 모두 한 번 일어나는 상태 전환이다. hover·포커스 색 전환 80ms(`--t-fast`), 팝업 등장 120–140ms(4px 위에서), 셰브론 회전 150ms. 곡선은 `cubic-bezier(0.16, 1, 0.3, 1)`.

## Do's and Don'ts

### Do:
- **Do** 사용자가 누르는 주 동작과 모든 초점 표시에만 남색(`accent`, `focus`)을 쓴다.
- **Do** 맥락 엔진이 고르고, 붙이고, 인덱싱한 결과를 보여줄 때만 청록(`context`)을 쓰고, 면이 필요하면 `context-soft` 바탕 + `context-line` 테두리로 한다.
- **Do** 새 면은 `sunken`/`bg`/`editor-bg`/`elevated` 중 하나와 1px `border`로 나눈다.
- **Do** VS Code의 치수를 지킨다: 22px 목록 행, 36px 탭·뷰 제목, 24px 상태 표시줄, 48px 액티비티 바, 18px 뷰 안쪽 여백.
- **Do** 토큰·비용·위치·개수 숫자는 `tabular-nums`로 쓴다.
- **Do** 활성 상태는 2px 남색 막대(탭 위, 패널 탭 아래, 액티비티 바 왼쪽)로 표시한다.
- **Do** 다크와 라이트 모두에서 같은 역할 토큰을 쓴다. 라이트의 청록은 대비를 위해 #0b7f94로 어둡다.
- **Do** 새 반복 애니메이션이 필요해 보이면 대신 육각형 `working` 상태를 쓴다. 텍스트 스트리밍 커서의 깜박임은 편집기 커서 관례로 예외다.

### Don't:
- **Don't** 청록→남색 브랜드 그라데이션을 육각형 표시 밖(버튼, 배경, 테두리, 글자)에 쓴다.
- **Don't** 한 요소 안에서 남색과 청록을 섞는다.
- **Don't** 빛나는 테두리, 색 번짐 그림자, 네온 느낌의 채도 높은 면을 쓴다.
- **Don't** 워크벤치 면이나 채팅 카드에 그림자를 준다. 그림자는 떠 있는 층만의 것이다.
- **Don't** 순수 회색이나 흰색 투명 막을 새로 들인다. 막은 남색 기운 회색의 투명도로만 만든다.
- **Don't** VS Code와 다른 위치·단축키·크기로 기본 워크벤치 요소를 옮긴다.
- **Don't** 편집기 문법 색을 Lantern 색으로 바꾼다. 문법 색은 Dark+/Light+ 그대로다.
- **Don't** 외부 CDN 글꼴·아이콘을 불러온다. Pretendard와 Codicons는 번들되어 있다.
