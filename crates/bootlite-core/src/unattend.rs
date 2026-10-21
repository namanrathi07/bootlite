use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowsEdition {
    Pro,
    Home,
    HomeSingleLanguage,
    Enterprise,
    Education,
    PromptUser,
}

impl WindowsEdition {
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Pro => "Pro",
            Self::Home => "Home",
            Self::HomeSingleLanguage => "Home SL",
            Self::Enterprise => "Enterprise",
            Self::Education => "Education",
            Self::PromptUser => "Ask Me",
        }
    }

    pub fn full_label(self) -> &'static str {
        match self {
            Self::Pro => "Windows 10/11 Pro (Auto-Select)",
            Self::Home => "Windows 10/11 Home (Auto-Select)",
            Self::HomeSingleLanguage => "Windows 10/11 Home Single Language",
            Self::Enterprise => "Windows 10/11 Enterprise (Auto-Select)",
            Self::Education => "Windows 10/11 Education (Auto-Select)",
            Self::PromptUser => "Ask Me During Setup (Manual Edition Selection)",
        }
    }

    /// Official Microsoft Generic Setup Keys (KMS/Retail setup selection keys).
    /// These tell Windows Setup which edition image to install so it skips the Edition prompt
    /// and jumps straight to the Drive Format screen, without blocking OEM/Digital activation.
    pub fn generic_setup_key(self) -> Option<&'static str> {
        match self {
            Self::Pro => Some("W269N-WFGWX-YVC9B-4J6C9-T83GX"),
            Self::Home => Some("TX9XD-98N7V-6WMQ6-BX7FG-H8Q99"),
            Self::HomeSingleLanguage => Some("7HNRX-D7KGG-3K4RQ-4WPJ4-YTDFH"),
            Self::Enterprise => Some("NPPR9-FWDCX-D2C8J-H872K-2YT43"),
            Self::Education => Some("NW6C2-QMPVW-D7KKK-3GKT6-VCFB2"),
            Self::PromptUser => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowsUnattendConfig {
    /// Enabled by default so Windows Setup skips pre-disk prompts and jumps straight to the Drive Format screen
    pub enabled: bool,
    pub edition: WindowsEdition,
    pub ui_language: String,
    pub time_currency_locale: String,
    pub keyboard_locale: String,
    // AutoPilot Hardware Hash Extraction to USB (\AutoPilot\)
    pub extract_autopilot_hash: bool,
    // Basic Setup Tweaks
    pub bypass_hardware_checks: bool,
    pub bypass_ms_account: bool,
    pub disable_privacy_questions: bool,
    pub disable_bitlocker: bool,
    pub create_local_account: bool,
    pub local_username: String,
    pub local_password: String,
    pub auto_logon: bool,
    // Advanced & System Customization
    pub computer_name: String,
    pub remove_bloatware_widgets: bool,
    pub classic_context_menu_and_ext: bool,
    pub custom_firstlogon_cmd: String,
}

impl Default for WindowsUnattendConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            edition: WindowsEdition::Pro,
            ui_language: "en-US".to_string(),
            time_currency_locale: "en-IN".to_string(),
            keyboard_locale: "en-US".to_string(),
            extract_autopilot_hash: true,
            bypass_hardware_checks: true,
            bypass_ms_account: true,
            disable_privacy_questions: true,
            disable_bitlocker: true,
            create_local_account: false,
            local_username: "Admin".to_string(),
            local_password: String::new(),
            auto_logon: true,
            computer_name: String::new(),
            remove_bloatware_widgets: true,
            classic_context_menu_and_ext: true,
            custom_firstlogon_cmd: String::new(),
        }
    }
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Generates the standalone PowerShell script placed inside `\AutoPilot\Extract-AutoPilotHash.ps1` on the USB drive.
/// Saves one single clean Intune CSV per laptop (`AutoPilotHWID_<SerialNumber>.csv`) so hostname changes never duplicate files.
pub fn generate_autopilot_ps1_script() -> &'static str {
    r#"# BootLite Windows AutoPilot Hardware Hash Extractor
# Saves a single Microsoft Intune Hardware Hash CSV (AutoPilotHWID_<SerialNumber>.csv) in the USB \AutoPilot\ folder.

$ErrorActionPreference = 'Stop'

try {
    # 1. Locate the target AutoPilot output directory on the USB drive
    $OutDir =$PSScriptRoot
    if ([string]::IsNullOrWhiteSpace($OutDir) -or -not (Test-Path -LiteralPath$OutDir)) {
        $UsbDisk = Get-Volume \vert{} Where-Object {$_.DriveLetter -ne $null } \vert{} ForEach-Object {$Candidate = "$($_.DriveLetter):\AutoPilot"
            if (Test-Path -LiteralPath $Candidate) {$Candidate }
        } | Select-Object -First 1
        if ($UsbDisk) {
            $OutDir =$UsbDisk
        } else {
            $OutDir = "C:\AutoPilot"
        }
    }
    if (-not (Test-Path -LiteralPath $OutDir)) {
        New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
    }

    # 2. Query BIOS Serial Number
    $Bios = Get-CimInstance -ClassName Win32_BIOS -ErrorAction SilentlyContinue
    $Serial = if ($Bios -and $Bios.SerialNumber) { ($Bios.SerialNumber).Trim() } else { "UNKNOWN_SERIAL" }
    $SafeSerial = ($Serial -replace '[^a-zA-Z0-9_-]', '_')

    # 3. Query Windows Product ID
    $ProdId = ""
    try {
        $ProdId = (Get-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -Name ProductId -ErrorAction SilentlyContinue).ProductId
    } catch {}

    # 4. Query 4K Hardware Hash from WMI MDM_DevDetail_Ext01
    $DevDetail = Get-CimInstance -Namespace 'root/cimv2/mdm/dmmap' -ClassName 'MDM_DevDetail_Ext01' -Filter "InstanceID='Ext' AND ParentID='./DevDetail'" -ErrorAction SilentlyContinue
    $HwHash = if ($DevDetail -and $DevDetail.DeviceHardwareData) { ($DevDetail.DeviceHardwareData).Trim() } else { "" }

    if ([string]::IsNullOrWhiteSpace($HwHash)) {
        Write-Warning "MDM_DevDetail_Ext01 Hardware Hash is not yet populated in WMI."
        exit 1
    }

    $CsvHeader = "Device Serial Number,Windows Product ID,Hardware Hash"
    $CsvRow    = "$Serial,$ProdId,$HwHash"

    # 5. Write ONE clean Intune-ready CSV file named by Serial Number (overwrites if run twice on same laptop)
    $DeviceFile = Join-Path $OutDir "AutoPilotHWID_${SafeSerial}.csv"
    Set-Content -LiteralPath $DeviceFile -Value @($CsvHeader,$CsvRow) -Encoding Ascii -Force

    Write-Host "[OK] Intune AutoPilot CSV saved to: $DeviceFile" -ForegroundColor Green
    exit 0
} catch {
    Write-Error "Failed to extract AutoPilot Hardware Hash: $_"
    exit 1
}
"#
}

/// Generates the one-click batch launcher placed at `\AutoPilot\Extract-AutoPilotHash.cmd` on the USB drive.
pub fn generate_autopilot_cmd_launcher() -> &'static str {
    "@echo off\r\n\
     title BootLite - Windows AutoPilot Hardware Hash Extractor\r\n\
     echo ======================================================================\r\n\
     echo   BootLite AutoPilot Hardware Hash Extractor\r\n\
     echo   Target Output Folder: %~dp0\r\n\
     echo ======================================================================\r\n\
     powershell.exe -NoProfile -ExecutionPolicy Bypass -File \"%~dp0Extract-AutoPilotHash.ps1\"\r\n\
     if %ERRORLEVEL% EQU 0 (\r\n\
         echo.\r\n\
         echo [SUCCESS] Intune Hardware Hash CSV saved inside %~dp0\r\n\
     ) else (\r\n\
         echo.\r\n\
         echo [WARN] Could not extract hash. Run as Administrator inside Windows OOBE\r\n\
         echo        ^(Shift+F10^) or full Windows desktop.\r\n\
     )\r\n\
     if \"%1\" NEQ \"--silent\" pause\r\n"
}

impl WindowsUnattendConfig {
    pub fn should_inject_unattend(&self) -> bool {
        self.enabled || self.extract_autopilot_hash
    }

    pub fn active_tweak_count(&self) -> usize {
        let mut count = 0;
        if self.extract_autopilot_hash {
            count += 1;
        }
        if self.enabled {
            count += 1; // Jump straight to Disk Format (skip Language/Key/EULA)
            if self.bypass_hardware_checks {
                count += 1;
            }
            if self.bypass_ms_account {
                count += 1;
            }
            if self.disable_privacy_questions {
                count += 1;
            }
            if self.disable_bitlocker {
                count += 1;
            }
            if self.create_local_account && !self.local_username.trim().is_empty() {
                count += 1;
            }
            if !self.computer_name.trim().is_empty() {
                count += 1;
            }
            if self.remove_bloatware_widgets {
                count += 1;
            }
            if self.classic_context_menu_and_ext {
                count += 1;
            }
            if !self.custom_firstlogon_cmd.trim().is_empty() {
                count += 1;
            }
        }
        count
    }

    pub fn generate_xml(&self) -> String {
        let ui_lang = if self.ui_language.trim().is_empty() {
            "en-US".to_string()
        } else {
            xml_escape(self.ui_language.trim())
        };
        let locale = if self.time_currency_locale.trim().is_empty() {
            ui_lang.clone()
        } else {
            xml_escape(self.time_currency_locale.trim())
        };
        let kbd = if self.keyboard_locale.trim().is_empty() {
            ui_lang.clone()
        } else {
            xml_escape(self.keyboard_locale.trim())
        };

        let mut pe_commands = Vec::new();
        if self.enabled && self.bypass_hardware_checks {
            for key in [
                "BypassTPMCheck",
                "BypassSecureBootCheck",
                "BypassRAMCheck",
                "BypassStorageCheck",
                "BypassCPUCheck",
            ] {
                pe_commands.push(format!(
                    r#"reg add "HKLM\SYSTEM\Setup\LabConfig" /v {} /t REG_DWORD /d 1 /f"#,
                    key
                ));
            }
        }

        let mut specialize_commands = Vec::new();
        if self.extract_autopilot_hash {
            specialize_commands.push(
                r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Get-Volume | Where-Object { $_.DriveLetter } \vert{} ForEach-Object {$s = \"$($_.DriveLetter):\AutoPilot\Extract-AutoPilotHash.ps1\"; if (Test-Path -LiteralPath $s) { &$s } }""#.to_string(),
            );
        }
        if self.enabled {
            if self.bypass_ms_account {
                specialize_commands.push(
                    r#"reg add "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\OOBE" /v BypassNRO /t REG_DWORD /d 1 /f"#.to_string(),
                );
            }
            if self.disable_bitlocker {
                specialize_commands.push(
                    r#"reg add "HKLM\SYSTEM\CurrentControlSet\Control\BitLocker" /v PreventDeviceEncryption /t REG_DWORD /d 1 /f"#.to_string(),
                );
            }
            if self.remove_bloatware_widgets {
                specialize_commands.push(
                    r#"reg add "HKLM\SOFTWARE\Policies\Microsoft\Dsh" /v AllowNewsAndInterests /t REG_DWORD /d 0 /f"#.to_string(),
                );
                specialize_commands.push(
                    r#"reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent" /v DisableWindowsConsumerFeatures /t REG_DWORD /d 1 /f"#.to_string(),
                );
            }
        }

        let mut firstlogon_commands = Vec::new();
        if self.extract_autopilot_hash {
            firstlogon_commands.push(
                r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Get-Volume | Where-Object { $_.DriveLetter } \vert{} ForEach-Object {$s = \"$($_.DriveLetter):\AutoPilot\Extract-AutoPilotHash.ps1\"; if (Test-Path -LiteralPath $s) { &$s } }""#.to_string(),
            );
        }
        if self.enabled {
            if self.classic_context_menu_and_ext {
                firstlogon_commands.push(
                    r#"reg add "HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32" /f /ve"#.to_string(),
                );
                firstlogon_commands.push(
                    r#"reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced" /v HideFileExt /t REG_DWORD /d 0 /f"#.to_string(),
                );
            }
            if self.remove_bloatware_widgets {
                firstlogon_commands.push(
                    r#"reg add "HKCU\Software\Policies\Microsoft\Windows\Explorer" /v DisableSearchBoxSuggestions /t REG_DWORD /d 1 /f"#.to_string(),
                );
            }
            if !self.custom_firstlogon_cmd.trim().is_empty() {
                firstlogon_commands.push(self.custom_firstlogon_cmd.trim().to_string());
            }
        }

        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        xml.push_str("<unattend xmlns=\"urn:schemas-microsoft-com:unattend\" xmlns:wcm=\"http://schemas.microsoft.com/WMIConfig/2002/State\">\n");

        // 1. windowsPE pass: Skips Language, Keyboard, Product Key, Edition & EULA -> Lands directly on the Drive Format screen!
        if self.enabled {
            xml.push_str("  <settings pass=\"windowsPE\">\n");
            xml.push_str("    <component name=\"Microsoft-Windows-International-Core-WinPE\" processorArchitecture=\"amd64\" publicKeyToken=\"31bf3856ad364e35\" language=\"neutral\" versionScope=\"nonSxS\">\n");
            xml.push_str("      <SetupUILanguage>\n");
            xml.push_str(&format!("        <UILanguage>{}</UILanguage>\n", ui_lang));
            xml.push_str("        <WillShowUI>OnError</WillShowUI>\n");
            xml.push_str("      </SetupUILanguage>\n");
            xml.push_str(&format!("      <UILanguage>{}</UILanguage>\n", ui_lang));
            xml.push_str(&format!("      <UserLocale>{}</UserLocale>\n", locale));
            xml.push_str(&format!("      <SystemLocale>{}</SystemLocale>\n", locale));
            xml.push_str(&format!("      <InputLocale>{}</InputLocale>\n", kbd));
            xml.push_str("    </component>\n");
            xml.push_str("    <component name=\"Microsoft-Windows-Setup\" processorArchitecture=\"amd64\" publicKeyToken=\"31bf3856ad364e35\" language=\"neutral\" versionScope=\"nonSxS\">\n");
            xml.push_str("      <UserData>\n");
            xml.push_str("        <AcceptEula>true</AcceptEula>\n");
            xml.push_str("        <ProductKey>\n");
            if let Some(gkey) = self.edition.generic_setup_key() {
                xml.push_str(&format!("          <Key>{}</Key>\n", gkey));
            }
            xml.push_str("          <WillShowUI>OnError</WillShowUI>\n");
            xml.push_str("        </ProductKey>\n");
            xml.push_str("      </UserData>\n");
            if !pe_commands.is_empty() {
                xml.push_str("      <RunSynchronous>\n");
                for (i, cmd) in pe_commands.iter().enumerate() {
                    xml.push_str(&format!(
                        "        <RunSynchronousCommand wcm:action=\"add\">\n          <Order>{}</Order>\n          <Path>{}</Path>\n        </RunSynchronousCommand>\n",
                        i + 1,
                        xml_escape(cmd)
                    ));
                }
                xml.push_str("      </RunSynchronous>\n");
            }
            xml.push_str("    </component>\n");
            xml.push_str("  </settings>\n");
        }

        // 2. specialize pass
        if !specialize_commands.is_empty()
            || (self.enabled && !self.computer_name.trim().is_empty())
        {
            xml.push_str("  <settings pass=\"specialize\">\n");
            if self.enabled && !self.computer_name.trim().is_empty() {
                let cname = xml_escape(self.computer_name.trim());
                xml.push_str("    <component name=\"Microsoft-Windows-Shell-Setup\" processorArchitecture=\"amd64\" publicKeyToken=\"31bf3856ad364e35\" language=\"neutral\" versionScope=\"nonSxS\">\n");
                xml.push_str(&format!("      <ComputerName>{}</ComputerName>\n", cname));
                xml.push_str("    </component>\n");
            }
            if !specialize_commands.is_empty() {
                xml.push_str("    <component name=\"Microsoft-Windows-Deployment\" processorArchitecture=\"amd64\" publicKeyToken=\"31bf3856ad364e35\" language=\"neutral\" versionScope=\"nonSxS\">\n");
                xml.push_str("      <RunSynchronous>\n");
                for (i, cmd) in specialize_commands.iter().enumerate() {
                    xml.push_str(&format!(
                        "        <RunSynchronousCommand wcm:action=\"add\">\n          <Order>{}</Order>\n          <Path>{}</Path>\n        </RunSynchronousCommand>\n",
                        i + 1,
                        xml_escape(cmd)
                    ));
                }
                xml.push_str("      </RunSynchronous>\n");
                xml.push_str("    </component>\n");
            }
            xml.push_str("  </settings>\n");
        }

        // 3. oobeSystem pass
        xml.push_str("  <settings pass=\"oobeSystem\">\n");
        xml.push_str("    <component name=\"Microsoft-Windows-Shell-Setup\" processorArchitecture=\"amd64\" publicKeyToken=\"31bf3856ad364e35\" language=\"neutral\" versionScope=\"nonSxS\">\n");
        if self.enabled {
            xml.push_str("      <OOBE>\n");
            xml.push_str("        <HideEULAPage>true</HideEULAPage>\n");
            if self.bypass_ms_account {
                xml.push_str("        <HideOnlineAccountScreens>true</HideOnlineAccountScreens>\n");
                xml.push_str("        <HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE>\n");
            }
            if self.disable_privacy_questions {
                xml.push_str("        <ProtectYourPC>3</ProtectYourPC>\n");
            }
            xml.push_str("      </OOBE>\n");

            if self.create_local_account && !self.local_username.trim().is_empty() {
                let uname = xml_escape(self.local_username.trim());
                let upass = xml_escape(&self.local_password);
                xml.push_str("      <UserAccounts>\n");
                xml.push_str("        <LocalAccounts>\n");
                xml.push_str("          <LocalAccount wcm:action=\"add\">\n");
                xml.push_str(&format!("            <Name>{}</Name>\n", uname));
                xml.push_str(&format!("            <DisplayName>{}</DisplayName>\n", uname));
                xml.push_str("            <Group>Administrators</Group>\n");
                xml.push_str("            <Password>\n");
                xml.push_str(&format!("              <Value>{}</Value>\n", upass));
                xml.push_str("              <PlainText>true</PlainText>\n");
                xml.push_str("            </Password>\n");
                xml.push_str("          </LocalAccount>\n");
                xml.push_str("        </LocalAccounts>\n");
                xml.push_str("      </UserAccounts>\n");

                if self.auto_logon {
                    xml.push_str("      <AutoLogon>\n");
                    xml.push_str("        <Enabled>true</Enabled>\n");
                    xml.push_str(&format!("        <Username>{}</Username>\n", uname));
                    xml.push_str("        <LogonCount>1</LogonCount>\n");
                    xml.push_str("        <Password>\n");
                    xml.push_str(&format!("          <Value>{}</Value>\n", upass));
                    xml.push_str("          <PlainText>true</PlainText>\n");
                    xml.push_str("        </Password>\n");
                    xml.push_str("      </AutoLogon>\n");
                }
            }
        }

        if !firstlogon_commands.is_empty() {
            xml.push_str("      <FirstLogonCommands>\n");
            for (i, cmd) in firstlogon_commands.iter().enumerate() {
                xml.push_str(&format!(
                    "        <SynchronousCommand wcm:action=\"add\">\n          <Order>{}</Order>\n          <CommandLine>{}</CommandLine>\n        </SynchronousCommand>\n",
                    i + 1,
                    xml_escape(cmd)
                ));
            }
            xml.push_str("      </FirstLogonCommands>\n");
        }

        xml.push_str("    </component>\n");
        xml.push_str("  </settings>\n");
        xml.push_str("</unattend>\n");
        xml
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autopilot_script_creates_single_clean_csv() {
        let ps1 = generate_autopilot_ps1_script();
        assert!(ps1.contains("Device Serial Number,Windows Product ID,Hardware Hash"));
        assert!(ps1.contains("MDM_DevDetail_Ext01"));
        assert!(ps1.contains("AutoPilotHWID_${SafeSerial}.csv"));
        assert!(!ps1.contains("AutoPilotHWID_Fleet.csv"));
    }

    #[test]
    fn test_unattend_xml_skips_to_disk_format_screen() {
        let cfg = WindowsUnattendConfig::default();
        assert!(cfg.enabled);
        let xml = cfg.generate_xml();
        assert!(xml.contains("<SetupUILanguage>"));
        assert!(xml.contains("<WillShowUI>OnError</WillShowUI>"));
        assert!(xml.contains("W269N-WFGWX-YVC9B-4J6C9-T83GX")); // Windows Pro Generic Setup Key
        assert!(xml.contains("BypassTPMCheck"));
    }
}
