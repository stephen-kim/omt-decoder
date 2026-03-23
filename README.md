# OMT Decoder (USB DAC Audio Fork)

한국어 문서는 [README.ko.md](README.ko.md)를 참고하세요.

This fork is intended to be installed on Raspberry Pi with three steps.

## 1. Set Raspberry Pi OS to Console Boot

`omtplayer` cannot render directly in desktop mode.

```bash
sudo raspi-config
```

Choose `1 System Options` -> `S5 Boot` -> `B1 Console Text console`.

## 2. Clone This Repository

```bash
git clone https://github.com/stephen-kim/omt-decoder.git
cd ~/omt-decoder
```

## 3. Run the Install Script

```bash
chmod +x build_and_install_service.sh
./build_and_install_service.sh
```

The script installs dependencies, installs .NET 8 if needed, builds the
project, deploys it to `/opt/omtplayer`, and registers the `omtplayer` systemd
service.

## Check Status

```bash
sudo systemctl status omtplayer
```

If the install succeeded, the web UI should be available at:

```text
http://<pi-ip>:8080/
```
