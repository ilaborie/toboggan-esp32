use anyhow::{bail, Result};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::{modem::Modem, peripheral};
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};
use log::info;

/// Initialize synchronous `WiFi` connection with provided credentials
///
/// # Errors
/// Returns error if `WiFi` initialization fails, SSID/password conversion fails,
/// scanning fails, or connection to the specified network fails
///
/// # Panics  
/// Panics if SSID or password cannot be converted to `WiFi` configuration format
#[allow(clippy::min_ident_chars)]
pub fn wifi_sync(
    ssid: &str,
    password: &str,
    modem: impl peripheral::Peripheral<P = Modem> + 'static,
    sysloop: EspSystemEventLoop,
) -> Result<Box<EspWifi<'static>>> {
    let mut auth_method = AuthMethod::WPA2Personal;
    if ssid.is_empty() {
        bail!("Missing WiFi name");
    }
    if password.is_empty() {
        auth_method = AuthMethod::None;
        info!("WiFi password is empty");
    }

    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), None)?;
    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;

    info!("Starting WiFi...");
    wifi.start()?;

    info!("Scanning for available WiFi networks...");
    let ap_infos = wifi.scan()?;

    // Log all discovered networks for debugging
    info!("📡 Found {} WiFi networks:", ap_infos.len());
    for (idx, ap) in ap_infos.iter().enumerate() {
        info!(
            "  [{}] SSID: '{}' | Channel: {} | Signal: {} dBm | Auth: {:?}",
            idx + 1,
            ap.ssid,
            ap.channel,
            ap.signal_strength,
            ap.auth_method
        );
    }

    let target_ap = ap_infos.into_iter().find(|ap| ap.ssid == ssid);
    let channel = if let Some(ref target_ap) = target_ap {
        info!(
            "✅ Found configured access point '{ssid}' on channel {} with signal strength {} dBm (Auth: {:?})",
            target_ap.channel,
            target_ap.signal_strength,
            target_ap.auth_method
        );
        Some(target_ap.channel)
    } else {
        info!("⚠️  Configured access point '{ssid}' not found during scanning, will attempt connection with unknown channel");
        None
    };

    info!("🔧 Configuring WiFi with SSID: '{ssid}', Auth: {:?}, Channel: {:?}", auth_method, channel);

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: ssid
            .try_into()
            .map_err(|()| anyhow::anyhow!("Failed to parse SSID '{}' into WiFi config (may be too long or contain invalid chars)", ssid))?,
        password: password
            .try_into()
            .map_err(|()| anyhow::anyhow!("Failed to parse password into WiFi config (may be too long or contain invalid chars)"))?,
        channel,
        auth_method,
        ..Default::default()
    }))?;

    info!("🔌 Attempting to connect to '{ssid}'...");
    wifi.connect()
        .map_err(|e| anyhow::anyhow!("Connection to '{}' failed: {} (Check SSID/password, signal strength, or AP availability)", ssid, e))?;

    info!("⏳ Waiting for DHCP lease...");
    wifi.wait_netif_up()
        .map_err(|e| anyhow::anyhow!("Failed to obtain DHCP lease: {} (Check DHCP server availability)", e))?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
    info!("✅ WiFi connected successfully!");
    info!("   IP Address:      {}", ip_info.ip);
    info!("   Subnet Mask:     {}", ip_info.subnet.mask);
    info!("   Gateway:         {}", ip_info.subnet.gateway);
    if let Some(dns) = ip_info.dns {
        info!("   DNS (Primary):   {dns}");
    }
    if let Some(secondary_dns) = ip_info.secondary_dns {
        info!("   DNS (Secondary): {secondary_dns}");
    }

    Ok(Box::new(esp_wifi))
}
