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

After installing the prerequisites, you can build and install the service with
a single script:

```bash
cd ~/omt-decoder
chmod +x build_and_install_service.sh
./build_and_install_service.sh
```

The script runs `apt update`, installs required packages, installs .NET 8, then
deploys to `/opt/omtplayer` and enables the `omtplayer` systemd service.

## Main Changes

- Added ALSA-based audio output with automatic USB audio device detection
- Added selectable audio output devices in the web UI with immediate apply
- Included OMT audio frame receive and playback handling

## Requirements

- Raspberry Pi 5 (base OS; 2GB model also works)
- .NET 8
- clang
- git
- libomtnet
- libvmx

## Performance and Format Notes

- Can decode 1080p60 on a Raspberry Pi 5 2GB
- Automatically matches the display's supported resolution
- Prefers 60Hz when there is no exact frame rate match
- Outputs interlaced sources as progressive without deinterlacing

## Installation and Build

Install Raspberry Pi OS first, then continue.

1. Update package lists

```bash
sudo apt update
```

2. Switch Raspberry Pi OS to console boot

`omtplayer` cannot render directly in desktop mode.

```bash
sudo raspi-config
```

Choose `1 System Options` -> `S5 Boot` -> `B1 Console Text console`.

3. Install .NET 8

```bash
curl -sSL https://dot.net/v1/dotnet-install.sh | bash /dev/stdin --channel 8.0

echo 'export DOTNET_ROOT=$HOME/.dotnet' >> ~/.bashrc
echo 'export PATH=$PATH:$HOME/.dotnet' >> ~/.bashrc
source ~/.bashrc
```

Reference: https://learn.microsoft.com/en-us/dotnet/iot/deployment  
Important: the `--channel` value must be `8.0`.

4. Install clang

```bash
sudo apt install clang
```

5. Arrange the source tree

Assume the cloned repository is laid out like this:

```text
~/omt-decoder/libvmx
~/omt-decoder/libomtnet
~/omt-decoder/omtplayer
```

Clone `libvmx` and `libomtnet` from the original repositories, and use this fork
as `~/omt-decoder`.

6. Build `libvmx`

```bash
cd ~/omt-decoder/libvmx/build
chmod 755 buildlinuxarm64.sh
./buildlinuxarm64.sh
```

7. Build `libomtnet`

```bash
cd ~/omt-decoder/libomtnet/build
chmod 755 buildall.sh
./buildall.sh
```

8. Build `omtplayer`

```bash
cd ~/omt-decoder/omtplayer/build
chmod 755 buildlinuxarm64.sh
./buildlinuxarm64.sh
```

9. Output location

After the build, binaries are generated under
`~/omt-decoder/omtplayer/build/arm64`.

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

## Register as a Service

If you want the player to start automatically on boot:

1. Copy the executable files

```bash
sudo mkdir /opt/omtplayer
sudo cp ~/omt-decoder/omtplayer/build/arm64/* /opt/omtplayer/
```

2. Install the systemd service

```bash
sudo cp ~/omt-decoder/omtplayer/omtplayer.service /etc/systemd/system/
```

3. Enable and start the service

```bash
sudo systemctl daemon-reload
sudo systemctl enable omtplayer
sudo systemctl start omtplayer
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
