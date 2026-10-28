<img width="120" alt="Group 1" src="https://github.com/user-attachments/assets/93205710-33fc-4bfa-9893-87ac530ab405" />


# CleanKakao

CleanKakao는 Windows용 KakaoTalk의 배너 및 팝업 광고를 없애주는 시스템 트레이 앱입니다.

## 스크린샷

<img width="600" alt="cleankakao" src="https://github.com/user-attachments/assets/7063895d-1c4e-4ea9-a418-f872f47ebeb0" />

## 다운로드

최신 버전은 [GitHub Releases](https://github.com/ghostface2232/cleankakao/releases/latest)에서 확인할 수 있습니다.

## 기능

- KakaoTalk PC 메인 창의 배너 광고 영역 숨김
- 광고가 사라진 자리에 대화 목록/콘텐츠 영역 확장
- 트레이 아이콘에서 차단 켜기/끄기
- 설정 창에서 광고 차단, 자동 시작, 업데이트 확인 설정
- GitHub Releases 기반 업데이트 확인·알림 및 앱 내 다운로드·적용·재시작

## 설정 윈도우 및 트레이 아이콘

<img width="400" alt="cleankakao_setting" src="https://github.com/user-attachments/assets/8633950a-f737-4abc-b3dc-5c6c0e6bb7e2" />


## 사용법

1. Releases에서 `cleankakao-v*-x86_64.zip`을 다운로드합니다.
2. 압축을 풀고 `cleankakao.exe`를 실행합니다.
3. 트레이 아이콘 메뉴에서 차단 상태를 전환하거나 설정 창을 엽니다.
4. 자동 시작이 필요하면 설정 창에서 활성화합니다.
5. 설정 창 하단의 `업데이트 확인`을 누르고, 새 버전이 있으면 `업데이트 후 재시작`을 누릅니다. 다운로드와 SHA-256 검증 후 실행 파일을 교체하고 설정 창을 다시 엽니다.

업데이트는 사용자가 버튼을 눌렀을 때만 설치됩니다. `자동 업데이트 확인`을 끄면 자동 확인을 중단하며, 수동 확인은 계속 사용할 수 있습니다. 실행 파일이 있는 폴더에 쓰기 권한이 필요합니다. 파일 교체나 새 실행 파일 시작에 실패하면 기존 실행 파일을 복원합니다.

## 업데이트 릴리스

`v<Cargo.toml 버전>` 태그를 푸시하면 테스트와 Windows 빌드 후 초안 릴리스를 생성합니다. 릴리스 노트를 검토하고 게시해야 앱에서 감지합니다. ZIP 배포와 함께 `cleankakao-windows-x86_64.exe`를 업로드하며, 인앱 업데이트는 실행 중인 프로세스 아키텍처와 일치하는 EXE 및 GitHub의 `sha256` digest가 있는 경우에만 제공됩니다. 게시한 릴리스의 파일은 교체하지 않고 새 버전으로 배포합니다.

기존의 브라우저 안내 방식 버전에서는 이 기능이 포함된 버전을 한 번 수동으로 설치해야 합니다. 이후 버전부터 인앱 업데이트를 사용할 수 있습니다.

## 빌드

필수 조건:
- Rust 1.95 이상
- Windows 10/11
- MSVC toolchain
- Windows SDK resource compiler

개발 실행:
```powershell
cargo run
```

릴리스 빌드:
```powershell
cargo build --release --target x86_64-pc-windows-msvc
```

테스트:
```powershell
cargo test
```

## 참고

CleanKakao는 [blurfx/KakaoTalkAdBlock](https://github.com/blurfx/KakaoTalkAdBlock)의 아이디어와 구현 접근에서 영향을 받았습니다.
CleanKakao는 별도의 Rust 구현이며, 독립적인 구조와 코드베이스를 사용합니다.
CleanKakao는 Kakao Corp. 또는 KakaoTalk와 무관한 비공식 서드파티 도구입니다.

## 라이선스

MIT License.
자세한 내용은 [LICENSE](LICENSE)를 참고하세요.
