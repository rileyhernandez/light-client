# light-client

`light-client` is an embedded Rust application designed to run on microcontrollers (specifically the Raspberry Pi Pico W) to control lights via hardware relays. It connects to your local Wi-Fi network and communicates with a central MQTT broker to receive commands and report its status.

This project is built to work in tandem with `light-server`, allowing users to remotely turn lights on and off.

## Features

- **Embedded Rust**: Built using the `embassy` framework for efficient, asynchronous execution on Cortex-M microcontrollers.
- **Wi-Fi Connectivity**: Uses the `cyw43` driver to connect to a local WPA2 Wi-Fi network.
- **MQTT Integration**: Subscribes to command topics and publishes state changes via MQTT (using `rust-mqtt`).
- **Relay Control**: Toggles a specified GPIO pin to turn a hardware relay (and the connected light) ON or OFF.

## Hardware Requirements

- **Raspberry Pi Pico W** (or a compatible RP2040 board with the CYW43439 Wi-Fi chip).
- **Relay Module** connected to a GPIO pin.
- Power supply suitable for the Pico W and the relay.

## Software Dependencies

- Rust toolchain (`stable` or `nightly` depending on your `rust-toolchain.toml`).
- `flip-link` and `probe-run` (or similar tools like `probe-rs`) for flashing the firmware.
- Access to an MQTT Broker (e.g., Mosquitto).

## Configuration

The application is configured at compile time using environment variables. You must provide these variables when building the project.

| Environment Variable | Description | Example |
| :--- | :--- | :--- |
| `RELAY_PIN` | The GPIO pin number connected to the relay. | `15` |
| `WIFI_SSID` | Your Wi-Fi network name. | `MyHomeNetwork` |
| `WIFI_PASSWORD` | Your Wi-Fi network password. | `supersecret` |
| `MQTT_BROKER_HOST` | The IP address of your MQTT broker. | `192.168.1.50` |
| `MQTT_BROKER_PORT` | The port of your MQTT broker. | `1883` |
| `DEVICE_ID` | A unique identifier for this specific client device. | `living-room-light` |

## MQTT Topics

The client interacts with the following MQTT topics based on the `DEVICE_ID`:

- **Command Topic**: `cmnd/<DEVICE_ID>/power`
  - The client subscribes to this topic.
  - Send `ON` to turn the relay on.
  - Send `OFF` to turn the relay off.
- **Status Topic**: `stat/<DEVICE_ID>/power`
  - The client publishes its current state (`ON`, `OFF`, or `OFFLINE` via LWT) to this topic.

## Building and Flashing

You can build and flash the application to your device using Cargo. Make sure your environment variables are set.

```bash
# Example build command
RELAY_PIN=15 \
WIFI_SSID="MyHomeNetwork" \
WIFI_PASSWORD="supersecret" \
MQTT_BROKER_HOST="192.168.1.50" \
MQTT_BROKER_PORT=1883 \
DEVICE_ID="living-room-light" \
cargo run --release
```

*(Note: Adjust the runner in your `.cargo/config.toml` to use `probe-rs run` or your preferred flashing tool).*

## Architecture

- **`wifi_task`**: Manages the Wi-Fi chip (`cyw43`) background processing.
- **`net_task`**: Manages the Embassy IPv4 network stack.
- **`main`**: Initializes the hardware, connects to Wi-Fi, acquires an IP address (currently configured for a static IP within the source code, you may need to adjust `Ipv4Address`), and runs the main MQTT event loop.

