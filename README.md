# OMT Decoder (USB DAC Audio Fork)

한국어 문서는 [README.ko.md](README.ko.md)를 참고하세요.

This repository is a fork built to monitor a vMix output feed over Ethernet
using the Open Media Transport (OMT) protocol.

It is based on https://github.com/openmediatransport/omtplayer and extends it
to support USB DAC audio output alongside HDMI video on Raspberry Pi 5.

Test environment:
- Raspberry Pi 5 4GB model
- USB DAC: https://www.coupang.com/vp/products/8926093893?vendorItemId=93070255836&sourceType=MyCoupang_my_orders_list_product_title

## Quick Build and Service Install

This repository is intended to be installed with a single script:

```bash
cd ~/omt-decoder
chmod +x build_and_install_service.sh
./build_and_install_service.sh
```

The script runs `apt update`, installs required packages, installs .NET 8, then
deploys to `/opt/omtplayer` and enables the `omtplayer` systemd service.

## Before Running the Script

Install Raspberry Pi OS first, then switch it to console boot mode.

`omtplayer` cannot render directly in desktop mode.

```bash
sudo raspi-config
```

Choose `1 System Options` -> `S5 Boot` -> `B1 Console Text console`.

## Main Changes

- Added ALSA-based audio output with automatic USB audio device detection
- Added selectable audio output devices in the web UI with immediate apply
- Included OMT audio frame receive and playback handling

## What the Script Does

- Updates apt package lists
- Installs `clang`, `git`, `curl`, and `libasound2`
- Installs .NET 8 if `dotnet` is missing
- Builds `libvmx`, `libomtnet`, and `omtplayer`
- Installs files into `/opt/omtplayer`
- Registers and restarts the `omtplayer` systemd service

The repository is expected to be checked out as:

```text
~/omt-decoder/libvmx
~/omt-decoder/libomtnet
~/omt-decoder/omtplayer
```

## Performance and Format Notes

- Can decode 1080p60 on a Raspberry Pi 5 2GB
- Automatically matches the display's supported resolution
- Prefers 60Hz when there is no exact frame rate match
- Outputs interlaced sources as progressive without deinterlacing

## Running

```bash
~/omt-decoder/omtplayer/build/arm64/omtplayer
```

- Connect HDMI output to the Pi's HDMI 0 port, next to the USB-C power port.
- Access the web UI from another PC on the same network:

```text
http://<pi-ip>:8080/
```

`omtplayer` remembers the last selected source automatically.

## Using USB DAC Audio

1. Confirm the USB DAC is detected

```bash
cat /proc/asound/cards
```

You should see `USB-Audio`.

2. Check the runtime log for selected device messages

- `Found USB Audio Device: plughw:X,0`
- `Using Default Audio Device: default`

3. Select audio devices in the web UI

In the Audio Devices section, select the USB or default device to switch
immediately. Multiple selections are also supported, for example HDMI and USB
DAC at the same time.

## Service Status

The install script already registers and restarts the `omtplayer` service.

To check the current service status manually:

```bash
sudo systemctl status omtplayer
```

If everything is working, the web UI will be available on port 8080.

## Troubleshooting

- If audio does not play, make sure `libasound2` is installed:

```bash
sudo apt install libasound2
```

- If audio only plays over HDMI, check the runtime log to confirm USB device
  detection.
