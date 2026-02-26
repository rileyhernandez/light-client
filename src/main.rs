#![no_std]
#![no_main]

use core::str::FromStr;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Config as NetConfig, DhcpConfig, Ipv4Address, Ipv4Cidr, Stack, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIN_23, PIN_25, PIO0};
use embassy_rp::pio::{InterruptHandler as PioInterruptHandler, Pio};
use embassy_time::{Timer};
use rand_core::RngCore;
use rust_mqtt::client::client::MqttClient;
use rust_mqtt::client::client_config::ClientConfig;
use rust_mqtt::utils::rng_generator::CountingRng;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};
use rust_mqtt::packet::v5::publish_packet::QualityOfService;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
});

// this is to set the desired state of spinlock 31 for the HAL;
// needs unsafe because we're running with no checks before RAM initialization;
// this helps for warm resets
#[cortex_m_rt::pre_init]
unsafe fn before_main() {
    embassy_rp::pac::SIO.spinlock(31).write_value(1);
}

#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<
        'static,
        Output<'static, PIN_23>,
        PioSpi<'static, PIN_25, PIO0, 0, DMA_CH0>,
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> ! {
    stack.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Program start");
    let p = embassy_rp::init(Default::default());
    let mut led = Output::new(p.PIN_22, Level::Low);

    let fw = include_bytes!(".././43439A0.bin");
    let clm = include_bytes!(".././43439A0_clm.bin");
    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    unwrap!(spawner.spawn(wifi_task(runner)));

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::Performance)
        .await;

    let seed: u64 = RoscRng.next_u64();
    let wifi_ssid = env!("WIFI_SSID");
    let wifi_password = env!("WIFI_PASSWORD");
    const CLIENT_ID: &str = "node-0";

    let mut dhcp_config = DhcpConfig::default();
    dhcp_config.hostname = Some(heapless::String::from_str(CLIENT_ID).unwrap());
    // let net_config = NetConfig::dhcpv4(dhcp_config);
    // need to use this for now because pi-hole complicating things:
    let net_config = NetConfig::ipv4_static(embassy_net::StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 1, 100), 24), // Pick a free IP
        gateway: Some(Ipv4Address::new(192, 168, 1, 1)),               // Your router IP
        dns_servers: heapless::Vec::from_slice(&[
            Ipv4Address::new(8, 8, 8, 8) // Use Google DNS instead of router's (pi-hole) for now
        ]).unwrap(),
    });
    

    static STACK: StaticCell<Stack<cyw43::NetDriver<'static>>> = StaticCell::new();
    static RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
    let stack = &*STACK.init(Stack::new(
        net_device,
        net_config,
        RESOURCES.init(StackResources::<4>::new()),
        seed,
    ));

    unwrap!(spawner.spawn(net_task(stack)));

    info!("Joining WiFi...");
    let mut failures = 0;
    loop {
        match control.join_wpa2(wifi_ssid, wifi_password).await {
            Ok(_) => {
                info!("Join successful!");
                break
            }
            Err(e) => {
                failures += 1;
                if failures >= 5 {
                    panic!("Maximum retries exceeded. Device failed to join WiFi. Exiting...");
                }
                warn!("Join failed with status={}. Retrying in 5s...", e.status);
                embassy_time::Timer::after_secs(5).await;
            }
        }
    }

    info!("Waiting for DHCP...");
    loop {
        if let Some(config) = stack.config_v4() {
            info!("IP Address: {}", config.address);
            break;
        }
        Timer::after_millis(200).await;
    }
    
    let mut mqtt_rx_buffer = [0; 1024];
    let mut mqtt_tx_buffer = [0; 1024];

    loop {
        // making a fresh tcp socket for every connection attempt
        let mut rx_buffer = [0; 4096];
        let mut tx_buffer = [0; 4096];
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);

        let broker_host = env!("MQTT_BROKER_HOST");
        let port = env!("MQTT_BROKER_PORT").parse().expect("Port must be a number");
        let broker_ip = Ipv4Address::from_str(broker_host).expect("Invalid IP");
        let endpoint = (broker_ip, port);

        info!("Attempting TCP Connection...");
        if let Err(e) = socket.connect(endpoint).await {
            error!("TCP Connect error: {:?}. Retrying in 5s...", e);
            Timer::after_secs(5).await;
            continue;
        }
        info!("TCP Connected.");

        let mut config: ClientConfig<'_, 5, CountingRng> = ClientConfig::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(seed),
        );
        config.add_client_id(CLIENT_ID);
        config.add_will(
            "stat/node-0/power",
            "OFFLINE".as_bytes(),
            false,
        );

        let mut client = MqttClient::new(
            socket,
            &mut mqtt_tx_buffer, 1024,
            &mut mqtt_rx_buffer, 1024,
            config,
        );

        info!("Connecting to MQTT Broker...");
        if let Err(e) = client.connect_to_broker().await {
            error!("MQTT Connect error: {:?}. Retrying...", e);
            Timer::after_secs(5).await;
            continue;
        }

        // 3. Subscriptions
        let command_topic = format!("cmnd/{}/power", env!("DEVICE_ID"));
        let status_topic = format!("stat/{}/power", env!("DEVICE_ID"));
        
        if let Err(e) = client.subscribe_to_topic(command_topic).await {
            error!("Subscribe error: {:?}", e);
            continue; 
        }
        
        // Publish initial state so server knows we are online
        let _ = client.send_message(status_topic, "OFF".as_bytes(), QualityOfService::QoS1, false).await;
        
        info!("MQTT Ready. Listening for commands...");

        // 4. Inner Message Processing Loop
        loop {
            // receive_message() handles PINGs internally based on set_keep_alive
            match client.receive_message().await {
                Ok((topic, payload)) => {
                    let msg = core::str::from_utf8(payload).unwrap_or("").trim();
                    info!("Topic: {}, Payload: {}", topic, msg);

                    match msg {
                        "ON" => {
                            led.set_high();
                            control.gpio_set(0, true).await;
                            let _ = client.send_message(status_topic, "ON".as_bytes(), QualityOfService::QoS1, false).await;
                        }
                        "OFF" => {
                            led.set_low();
                            control.gpio_set(0, false).await;
                            let _ = client.send_message(status_topic, "OFF".as_bytes(), QualityOfService::QoS1, false).await;
                        }
                        _ => warn!("Unknown command: {}", msg),
                    }
                }
                Err(e) => {
                    warn!("Connection lost: {:?}. Reconnecting...", e);
                    break; // Break inner loop to trigger outer loop reconnection
                }
            }
        }
        
        Timer::after_secs(1).await;
    }
}