/// Armazenamento local de credenciais de sites (para auto-login em áreas de
/// membros como MemberKit). Guardado em app_config_dir/credentials.json — nunca
/// versionado. Uso pessoal; não é um cofre criptografado.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SiteCredential {
    /// Host do site (ex. "potenza.memberkit.com.br"). Casa por sufixo.
    pub host: String,
    pub email: String,
    pub password: String,
}

fn creds_file(config_dir: &Path) -> PathBuf {
    config_dir.join("credentials.json")
}

/// Lê todas as credenciais salvas (lista vazia se o arquivo não existir).
pub fn load_all(config_dir: &Path) -> Vec<SiteCredential> {
    let path = creds_file(config_dir);
    match std::fs::read_to_string(&path) {
        Ok(txt) => serde_json::from_str(&txt).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Persiste a lista completa de credenciais.
pub fn save_all(config_dir: &Path, list: &[SiteCredential]) -> Result<(), String> {
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("Erro ao criar dir de config: {}", e))?;
    let path = creds_file(config_dir);
    let txt =
        serde_json::to_string_pretty(list).map_err(|e| format!("Erro ao serializar: {}", e))?;
    std::fs::write(&path, txt).map_err(|e| format!("Erro ao salvar credenciais: {}", e))?;
    Ok(())
}

/// Insere/atualiza a credencial de um host e salva.
pub fn upsert(config_dir: &Path, cred: SiteCredential) -> Result<(), String> {
    let mut list = load_all(config_dir);
    let host = cred.host.trim().to_lowercase();
    if let Some(existing) = list.iter_mut().find(|c| c.host.eq_ignore_ascii_case(&host)) {
        existing.email = cred.email;
        existing.password = cred.password;
    } else {
        list.push(SiteCredential {
            host,
            email: cred.email,
            password: cred.password,
        });
    }
    save_all(config_dir, &list)
}

/// Remove a credencial de um host.
pub fn remove(config_dir: &Path, host: &str) -> Result<(), String> {
    let mut list = load_all(config_dir);
    list.retain(|c| !c.host.eq_ignore_ascii_case(host));
    save_all(config_dir, &list)
}

/// Procura a credencial cujo host é sufixo do host informado (ou vice-versa),
/// para casar "potenza.memberkit.com.br" com uma credencial de "memberkit.com.br".
pub fn find_for_host(config_dir: &Path, host: &str) -> Option<SiteCredential> {
    let host = host.to_lowercase();
    let list = load_all(config_dir);
    list.into_iter().find(|c| {
        let h = c.host.to_lowercase();
        !h.is_empty() && (host == h || host.ends_with(&format!(".{}", h)) || host.ends_with(&h))
    })
}

/// Extrai o host de uma URL.
pub fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
}
