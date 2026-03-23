# OMT Decoder (USB DAC 오디오 지원 포크)

이 리포지토리는 vMix 송출 화면을 이더넷 연결 환경에서
Open Media Transport(OMT) 프로토콜로 모니터링하기 위해 만들었습니다.

이 저장소는 https://github.com/openmediatransport/omtplayer 를 기반으로,
Raspberry Pi 5에서 HDMI 영상과 함께 USB DAC 오디오 출력까지 지원하도록 확장한 포크입니다.

테스트 환경:
- Raspberry Pi 5 4GB 모델
- USB DAC: https://www.coupang.com/vp/products/8926093893?vendorItemId=93070255836&sourceType=MyCoupang_my_orders_list_product_title

## 빠른 빌드 + 서비스 등록 (스크립트)

이 저장소는 아래 스크립트 하나로 설치하는 것을 기준으로 합니다.

```bash
cd ~/omt-decoder
chmod +x build_and_install_service.sh
./build_and_install_service.sh
```

스크립트는 `apt update`, 필수 패키지 설치, dotnet 8 설치까지 수행한 뒤
`/opt/omtplayer`로 설치하고 `omtplayer` systemd 서비스를 활성화합니다.

## 스크립트 실행 전

라즈베리파이 OS를 먼저 설치한 뒤, 콘솔 부팅으로 바꿔야 합니다.

omtplayer는 데스크톱 모드에서 직접 출력이 불가능합니다.

```bash
sudo raspi-config
```

`1 System Options` → `S5 Boot` → `B1 Console Text console` 선택

## 주요 변경사항

- ALSA 기반 오디오 출력 추가 (USB Audio Device 자동 감지)
- 웹 UI에서 오디오 출력 장치 선택 가능 (복수 선택 가능, 즉시 적용)
- OMT Audio 프레임 수신 및 재생 처리 포함

## 스크립트가 하는 일

- apt 패키지 목록 업데이트
- `clang`, `git`, `curl`, `libasound2` 설치
- `dotnet`이 없으면 .NET 8 설치
- `libvmx`, `libomtnet`, `omtplayer` 빌드
- `/opt/omtplayer`로 파일 설치
- `omtplayer` systemd 서비스 등록 및 재시작

저장소는 아래 구조로 체크아웃되어 있다고 가정합니다.

```text
~/omt-decoder/libvmx
~/omt-decoder/libomtnet
~/omt-decoder/omtplayer
```

## 성능/형식

- 2GB Raspberry Pi 5 기준 1080p60 디코딩 가능
- 디스플레이가 지원하는 해상도에 맞춰 자동 매칭
- 프레임레이트는 60Hz를 우선 선택 (정확 매치가 없을 경우)
- 인터레이스 소스는 디인터레이싱 없이 프로그레시브로 출력

## 실행 방법

```bash
~/omt-decoder/omtplayer/build/arm64/omtplayer
```

- HDMI 출력은 Pi의 HDMI 0 포트(USB-C 전원 포트 옆)에 연결해야 합니다.
- 같은 네트워크의 다른 PC에서 웹 UI 접속:

```
http://<pi-ip>:8080/
```

omtplayer는 마지막으로 선택한 소스를 자동으로 기억합니다.

## USB DAC 오디오 사용

1. USB DAC 연결 후 인식 확인

```bash
cat /proc/asound/cards
```

`USB-Audio`가 표시되어야 합니다.

2. 실행 로그에서 장치 선택 확인

- `Found USB Audio Device: plughw:X,0`
- 또는 `Using Default Audio Device: default`

3. 웹 UI에서 오디오 장치 선택

웹 UI의 Audio Devices 섹션에서 USB/Default 장치를 선택하면 즉시 전환됩니다.
복수 선택도 가능합니다 (예: HDMI + USB DAC 동시 출력).

## 서비스 상태 확인

설치 스크립트가 이미 `omtplayer` 서비스를 등록하고 재시작합니다.

수동으로 상태만 확인하려면:

```bash
sudo systemctl status omtplayer
```

정상 동작 시 포트 8080에서 웹 UI가 접근 가능합니다.

## 문제 해결

- 소리가 안 나면 `libasound2` 설치 여부 확인:

```bash
sudo apt install libasound2
```

- HDMI로만 소리가 나오면 실행 로그에서 USB 장치 감지 여부를 확인하세요.
