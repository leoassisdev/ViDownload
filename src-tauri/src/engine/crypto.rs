use aes::cipher::{BlockDecryptMut, KeyIvInit};
use cbc::Decryptor;

type Aes128CbcDec = Decryptor<aes::Aes128>;

/// Decripta dados AES-128-CBC com PKCS7 padding
pub fn decrypt_aes128_cbc(data: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Result<Vec<u8>, String> {
    let mut buf = data.to_vec();
    let decryptor = Aes128CbcDec::new(key.into(), iv.into());

    let decrypted = decryptor
        .decrypt_padded_mut::<aes::cipher::block_padding::Pkcs7>(&mut buf)
        .map_err(|e| format!("Erro AES: {}", e))?;

    Ok(decrypted.to_vec())
}

/// Gera IV a partir do sequence number (padrão HLS quando IV não é especificado)
pub fn iv_from_sequence(sequence: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..16].copy_from_slice(&sequence.to_be_bytes());
    iv
}

/// Parse IV hex string do formato 0x...
pub fn parse_iv_hex(hex_str: &str) -> Result<[u8; 16], String> {
    let hex = hex_str
        .strip_prefix("0x")
        .or_else(|| hex_str.strip_prefix("0X"))
        .unwrap_or(hex_str);

    if hex.len() != 32 {
        return Err(format!("IV inválido: esperado 32 hex chars, recebeu {}", hex.len()));
    }

    let mut iv = [0u8; 16];
    for i in 0..16 {
        iv[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|e| format!("IV hex inválido: {}", e))?;
    }

    Ok(iv)
}

/// Busca a chave AES de uma URL
pub async fn fetch_key(client: &reqwest::Client, key_url: &str) -> Result<[u8; 16], String> {
    let response = client
        .get(key_url)
        .send()
        .await
        .map_err(|e| format!("Erro ao buscar chave: {}", e))?;

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Erro ao ler chave: {}", e))?;

    if bytes.len() != 16 {
        return Err(format!("Chave inválida: esperado 16 bytes, recebeu {}", bytes.len()));
    }

    let mut key = [0u8; 16];
    key.copy_from_slice(&bytes);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iv_from_sequence() {
        let iv = iv_from_sequence(0);
        assert_eq!(iv, [0u8; 16]);

        let iv = iv_from_sequence(1);
        assert_eq!(iv[15], 1);
        assert_eq!(iv[14], 0);

        let iv = iv_from_sequence(256);
        assert_eq!(iv[14], 1);
        assert_eq!(iv[15], 0);
    }

    #[test]
    fn test_parse_iv_hex() {
        let iv = parse_iv_hex("0x00000000000000000000000000000001").unwrap();
        assert_eq!(iv[15], 1);

        let iv = parse_iv_hex("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").unwrap();
        assert_eq!(iv, [0xFF; 16]);
    }

    #[test]
    fn test_decrypt_aes128_cbc() {
        // Chave e IV conhecidos
        let key = [0u8; 16];
        let iv = [0u8; 16];

        // Encriptar dados de teste para validar decriptação
        use aes::cipher::BlockEncryptMut;
        type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

        let plaintext = b"Hello AES-128!!!"; // 16 bytes (1 bloco)
        let mut buf = [0u8; 32]; // espaço para padding
        buf[..16].copy_from_slice(plaintext);

        let encryptor = Aes128CbcEnc::new(&key.into(), &iv.into());
        let encrypted = encryptor
            .encrypt_padded_mut::<aes::cipher::block_padding::Pkcs7>(&mut buf, 16)
            .unwrap();

        let decrypted = decrypt_aes128_cbc(encrypted, &key, &iv).unwrap();
        assert_eq!(&decrypted, plaintext);
    }
}
