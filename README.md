# OMT Decoder (USB DAC 오디오 지원 포크)

이 저장소는 https://github.com/openmediatransport/omtplayer 를 기반으로,
Raspberry Pi 5에서 HDMI 영상과 함께 USB DAC 오디오 출력까지 지원하도록 확장한 포크입니다.

## 빠른 빌드 + 서비스 등록 (스크립트)

사전 요구사항을 모두 설치한 뒤, 아래 스크립트 하나로 빌드와 서비스 등록까지 진행합니다.

```bash
cd ~/cpm-omt-decode
chmod +x build_and_install_service.sh
./build_and_install_service.sh
```

스크립트는 `apt update`, 필수 패키지 설치, dotnet 8 설치까지 수행한 뒤
`/opt/omtplayer`로 설치하고 `omtplayer` systemd 서비스를 활성화합니다.

## 주요 변경사항

- ALSA 기반 오디오 출력 추가 (USB Audio Device 자동 감지)
- 웹 UI에서 오디오 출력 장치 선택 가능 (복수 선택 가능, 즉시 적용)
- OMT Audio 프레임 수신 및 재생 처리 포함

## 요구사항

- Raspberry Pi 5 (기본 OS, 2GB 모델도 가능)
- dotnet 8
- clang
- git
- libomtnet
- libvmx

## 성능/형식

- 2GB Raspberry Pi 5 기준 1080p60 디코딩 가능
- 디스플레이가 지원하는 해상도에 맞춰 자동 매칭
- 프레임레이트는 60Hz를 우선 선택 (정확 매치가 없을 경우)
- 인터레이스 소스는 디인터레이싱 없이 프로그레시브로 출력

## 설치 및 빌드 (원본 README 기반)

1. 패키지 목록 업데이트

```bash
sudo apt update
```

2. Raspberry Pi OS를 콘솔 부팅으로 변경

omtplayer는 데스크톱 모드에서 직접 출력이 불가능합니다.

```bash
sudo raspi-config
```

`1 System Options` → `S5 Boot` → `B1 Console Text console` 선택

3. dotnet 8 설치

```bash
curl -sSL https://dot.net/v1/dotnet-install.sh | bash /dev/stdin --channel 8.0

echo 'export DOTNET_ROOT=$HOME/.dotnet' >> ~/.bashrc
echo 'export PATH=$PATH:$HOME/.dotnet' >> ~/.bashrc
source ~/.bashrc
```

참고: https://learn.microsoft.com/en-us/dotnet/iot/deployment  
중요: `--channel` 값은 `8.0`이어야 합니다.

4. clang 설치

```bash
sudo apt install clang
```

5. 소스 코드 배치

클론한 저장소 기준으로 아래 구조를 가정합니다.

```
~/cpm-omt-decode/libvmx
~/cpm-omt-decode/libomtnet
~/cpm-omt-decode/omtplayer
```

원본 저장소에서 `libvmx`, `libomtnet`를 클론하고,
본 포크를 `~/cpm-omt-decode`로 사용합니다.

6. libvmx 빌드

```bash
cd ~/cpm-omt-decode/libvmx/build
chmod 755 buildlinuxarm64.sh
./buildlinuxarm64.sh
```

7. libomtnet 빌드

```bash
cd ~/cpm-omt-decode/libomtnet/build
chmod 755 buildall.sh
./buildall.sh
```

8. omtplayer 빌드

```bash
cd ~/cpm-omt-decode/omtplayer/build
chmod 755 buildlinuxarm64.sh
./buildlinuxarm64.sh
```

9. 실행 파일 위치

빌드 후 결과물은 `~/cpm-omt-decode/omtplayer/build/arm64`에 생성됩니다.

## 실행 방법

```bash
~/cpm-omt-decode/omtplayer/build/arm64/omtplayer
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

## 서비스로 등록 (선택)

부팅 시 자동 실행이 필요하다면 다음을 수행하세요.

1. 실행 파일 복사

```bash
sudo mkdir /opt/omtplayer
sudo cp ~/cpm-omt-decode/omtplayer/build/arm64/* /opt/omtplayer/
```

2. systemd 서비스 등록

```bash
sudo cp ~/cpm-omt-decode/omtplayer/omtplayer.service /etc/systemd/system/
```

3. 서비스 활성화 및 시작

```bash
sudo systemctl daemon-reload
sudo systemctl enable omtplayer
sudo systemctl start omtplayer
sudo systemctl status omtplayer
```

정상 동작 시 포트 8080에서 웹 UI가 접근 가능합니다.

## 문제 해결

- 소리가 안 나면 `libasound2` 설치 여부 확인:

```bash
sudo apt install libasound2
```

- HDMI로만 소리가 나오면 실행 로그에서 USB 장치 감지 여부를 확인하세요.
