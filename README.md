# CleanKakao

CleanKakao는 Windows용 KakaoTalk의 배너 및 팝업 광고를 없애주는 시스템 트레이 앱입니다.

## 미리보기

<img width="400" alt="cleankakao" src="https://github.com/user-attachments/assets/ef2432a2-ee4c-4c86-9c21-20eb4496d5ea" />


## 다운로드

최신 버전은 [GitHub Releases](https://github.com/ghostface2232/cleankakao/releases/latest)에서 확인할 수 있습니다.

## 기능

- KakaoTalk PC 메인 창의 배너 광고 영역 숨김
- 광고가 사라진 자리에 대화 목록/콘텐츠 영역 확장
- 트레이 아이콘에서 차단 켜기/끄기
- 설정 창에서 광고 차단, 자동 시작, 업데이트 확인 설정
- GitHub Releases 기반 업데이트 확인 및 Windows Toast 알림

## 사용법

1. Releases에서 `cleankakao-v*-x86_64.zip`을 다운로드합니다.
2. 압축을 풀고 `cleankakao.exe`를 실행합니다.
3. 트레이 아이콘 메뉴에서 차단 상태를 전환하거나 설정 창을 엽니다.
4. 자동 시작이 필요하면 설정 창에서 활성화합니다.

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

## 참고 및 감사

CleanKakao는 [blurfx/KakaoTalkAdBlock](https://github.com/blurfx/KakaoTalkAdBlock)의 아이디어와 구현 접근에서 영향을 받았습니다.
CleanKakao는 별도의 Rust 구현이며, 독립적인 구조와 코드베이스를 사용합니다.

## 라이선스

MIT License.
자세한 내용은 [LICENSE](LICENSE)를 참고하세요.

## 면책 조항

CleanKakao는 Kakao Corp. 또는 KakaoTalk와 무관한 비공식 서드파티 도구입니다.
