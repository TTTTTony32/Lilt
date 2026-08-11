const SERVICE_NAME: &str = "com.lilt.app";

pub fn save_api_key(provider_id: &str, api_key: Option<&str>) -> Result<(), String> {
    #[cfg(windows)]
    {
        use keyring::Entry;
        let entry = Entry::new(SERVICE_NAME, provider_id)
            .map_err(|error| format!("打开 Windows 凭据存储失败：{error}"))?;
        match api_key.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => entry
                .set_password(value)
                .map_err(|error| format!("保存 API Key 失败：{error}")),
            None => entry.delete_credential().or_else(|error| {
                let message = error.to_string().to_lowercase();
                if message.contains("no entry") || message.contains("not found") {
                    Ok(())
                } else {
                    Err(format!("删除 API Key 失败：{error}"))
                }
            }),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (provider_id, api_key);
        Err("API Key 安全存储目前只支持 Windows 凭据管理器".to_string())
    }
}

pub fn load_api_key(provider_id: &str) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        use keyring::Entry;
        let entry = Entry::new(SERVICE_NAME, provider_id)
            .map_err(|error| format!("打开 Windows 凭据存储失败：{error}"))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                let message = error.to_string().to_lowercase();
                if message.contains("no entry") || message.contains("not found") {
                    Ok(None)
                } else {
                    Err(format!("读取 API Key 失败：{error}"))
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = provider_id;
        Err("API Key 安全存储目前只支持 Windows 凭据管理器".to_string())
    }
}
