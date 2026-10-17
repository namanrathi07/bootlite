use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowsUnattendConfig {
    pub enabled: bool,
    // Basic Tab (Rufus-style bypasses)
    pub bypass_hardware_checks: bool,
    pub bypass_ms_account: bool,
    pub disable_privacy_questions: bool,
    pub disable_bitlocker: bool,
    pub create_local_account: bool,
    pub local_username: String,
    pub local_password: String,
    pub auto_logon: bool,
    // Advanced Tab (Sysadmin & De-bloat)
    pub computer_name: String,
    pub ui_language: String,
    pub remove_bloatware_widgets: bool,
    pub classic_context_menu_and_ext: bool,
    pub custom_firstlogon_cmd: String,
}

impl Default for WindowsUnattendConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bypass_hardware_checks: true,
            bypass_ms_account: true,
            disable_privacy_questions: true,
            disable_bitlocker: true,
            create_local_account: false,
            local_username: "Admin".to_string(),
            local_password: String::new(),
            auto_logon: true,
            computer_name: String::new(),
            ui_language: "en-US".to_string(),
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

impl WindowsUnattendConfig {
    pub fn active_tweak_count(&self) -> usize {
        if !self.enabled {
            return 0;
        }
        let mut count = 0;
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
        count
    }

    pub fn generate_xml(&self) -> String {
        let lang = if self.ui_language.trim().is_empty() {
            "en-US".to_string()
        } else {
            xml_escape(self.ui_language.trim())
        };

        let mut pe_commands = Vec::new();
        if self.bypass_hardware_checks {
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

        let mut firstlogon_commands = Vec::new();
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

        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        xml.push_str("<unattend xmlns=\"urn:schemas-microsoft-com:unattend\" xmlns:wcm=\"http://schemas.microsoft.com/WMIConfig/2002/State\">\n");

        // 1. windowsPE pass
        xml.push_str("  <settings pass=\"windowsPE\">\n");
        xml.push_str("    <component name=\"Microsoft-Windows-International-Core-WinPE\" processorArchitecture=\"amd64\" publicKeyToken=\"31bf3856ad364e35\" language=\"neutral\" versionScope=\"nonSxS\">\n");
        xml.push_str(&format!("      <UILanguage>{}</UILanguage>\n", lang));
        xml.push_str(&format!("      <UserLocale>{}</UserLocale>\n", lang));
        xml.push_str(&format!("      <SystemLocale>{}</SystemLocale>\n", lang));
        xml.push_str(&format!("      <InputLocale>{}</InputLocale>\n", lang));
        xml.push_str("    </component>\n");
        xml.push_str("    <component name=\"Microsoft-Windows-Setup\" processorArchitecture=\"amd64\" publicKeyToken=\"31bf3856ad364e35\" language=\"neutral\" versionScope=\"nonSxS\">\n");
        xml.push_str("      <UserData>\n");
        xml.push_str("        <AcceptEula>true</AcceptEula>\n");
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

        // 2. specialize pass
        if !specialize_commands.is_empty() || !self.computer_name.trim().is_empty() {
            xml.push_str("  <settings pass=\"specialize\">\n");
            if !self.computer_name.trim().is_empty() {
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
