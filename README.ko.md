# OMT Decoder (USB DAC 오디오 지원 포크)

이 저장소는 라즈베리파이에서 아래 세 단계로 설치하는 것을 기준으로 합니다.

## 1. 라즈베리파이 OS를 콘솔 부팅으로 설정

omtplayer는 데스크톱 모드에서 직접 출력이 불가능합니다.

```bash
sudo raspi-config
```

`1 System Options` → `S5 Boot` → `B1 Console Text console` 선택

## 2. 저장소 클론

```bash
git clone https://github.com/stephen-kim/omt-decoder.git
cd ~/omt-decoder
```

## 3. 설치 스크립트 실행

```bash
chmod +x build_and_install_service.sh
./build_and_install_service.sh
```

스크립트가 의존성 설치, .NET 8 설치, 빌드, `/opt/omtplayer` 배포,
`omtplayer` systemd 서비스 등록까지 처리합니다.

## 상태 확인

```bash
sudo systemctl status omtplayer
```

설치가 정상적으로 끝났다면 웹 UI는 아래 주소로 접속할 수 있습니다.

```text
http://<pi-ip>:8080/
```
