/// Módulo de remux: TS → MP4 e fMP4 → MP4
/// Para MVP, usa concatenação direta de segmentos.
/// Segmentos TS são concatenados em um .ts que players modernos lêem.
/// Segmentos fMP4 são concatenados com init segment em um .mp4 válido.
///
/// Futuramente pode ser expandido com demux/mux real (parse de NAL units, AAC frames, etc.)

use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// Resultado do remux
pub struct RemuxResult {
    pub output_path: PathBuf,
    pub format: OutputFormat,
    pub size_bytes: u64,
}

#[derive(Debug, Clone)]
pub enum OutputFormat {
    TransportStream, // .ts concatenado
    FragmentedMp4,   // fMP4 com init + segments
}

/// Concatena segmentos TS em um único arquivo
pub async fn concat_ts_segments(
    segments_dir: &Path,
    output: &Path,
    segment_count: u64,
) -> Result<RemuxResult, String> {
    let mut file = tokio::fs::File::create(output)
        .await
        .map_err(|e| format!("Erro ao criar {}: {}", output.display(), e))?;

    let mut total_bytes: u64 = 0;

    for i in 0..segment_count {
        let seg_path = segments_dir.join(format!("seg_{:06}.ts", i));
        if seg_path.exists() {
            let data = tokio::fs::read(&seg_path)
                .await
                .map_err(|e| format!("Erro ao ler seg_{:06}: {}", i, e))?;
            total_bytes += data.len() as u64;
            file.write_all(&data)
                .await
                .map_err(|e| format!("Erro ao escrever seg_{:06}: {}", i, e))?;
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Erro ao finalizar: {}", e))?;

    Ok(RemuxResult {
        output_path: output.to_path_buf(),
        format: OutputFormat::TransportStream,
        size_bytes: total_bytes,
    })
}

/// Concatena segmentos fMP4 (init + media segments) em um único .mp4
pub async fn concat_fmp4_segments(
    segments_dir: &Path,
    output: &Path,
    segment_count: u64,
) -> Result<RemuxResult, String> {
    let mut file = tokio::fs::File::create(output)
        .await
        .map_err(|e| format!("Erro ao criar {}: {}", output.display(), e))?;

    let mut total_bytes: u64 = 0;

    // Init segment primeiro (contém ftyp + moov)
    let init_path = segments_dir.join("init.mp4");
    if init_path.exists() {
        let data = tokio::fs::read(&init_path)
            .await
            .map_err(|e| format!("Erro ao ler init.mp4: {}", e))?;
        total_bytes += data.len() as u64;
        file.write_all(&data)
            .await
            .map_err(|e| format!("Erro ao escrever init: {}", e))?;
    }

    // Media segments (contêm moof + mdat)
    for i in 0..segment_count {
        let seg_path = segments_dir.join(format!("seg_{:06}.ts", i));
        if seg_path.exists() {
            let data = tokio::fs::read(&seg_path)
                .await
                .map_err(|e| format!("Erro ao ler seg_{:06}: {}", i, e))?;
            total_bytes += data.len() as u64;
            file.write_all(&data)
                .await
                .map_err(|e| format!("Erro ao escrever seg_{:06}: {}", i, e))?;
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Erro ao finalizar: {}", e))?;

    Ok(RemuxResult {
        output_path: output.to_path_buf(),
        format: OutputFormat::FragmentedMp4,
        size_bytes: total_bytes,
    })
}

/// Parser básico de boxes ISO BMFF (MP4)
/// Retorna lista de (tipo, offset, tamanho) dos boxes de nível superior
pub fn parse_mp4_boxes(data: &[u8]) -> Vec<Mp4Box> {
    let mut boxes = Vec::new();
    let mut offset = 0;

    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;

        let box_type = std::str::from_utf8(&data[offset + 4..offset + 8])
            .unwrap_or("????")
            .to_string();

        let actual_size = if size == 0 {
            data.len() - offset // box até o fim
        } else if size == 1 && offset + 16 <= data.len() {
            // Extended size (64-bit)
            u64::from_be_bytes([
                data[offset + 8],
                data[offset + 9],
                data[offset + 10],
                data[offset + 11],
                data[offset + 12],
                data[offset + 13],
                data[offset + 14],
                data[offset + 15],
            ]) as usize
        } else {
            size
        };

        if actual_size < 8 || offset + actual_size > data.len() {
            break;
        }

        boxes.push(Mp4Box {
            box_type,
            offset,
            size: actual_size,
        });

        offset += actual_size;
    }

    boxes
}

#[derive(Debug, Clone)]
pub struct Mp4Box {
    pub box_type: String,
    pub offset: usize,
    pub size: usize,
}

/// Verifica se o arquivo é um fMP4 válido (tem ftyp/moov)
pub fn is_valid_fmp4_init(data: &[u8]) -> bool {
    let boxes = parse_mp4_boxes(data);
    let has_ftyp = boxes.iter().any(|b| b.box_type == "ftyp");
    let has_moov = boxes.iter().any(|b| b.box_type == "moov");
    has_ftyp && has_moov
}

/// Verifica se dados são um segmento fMP4 (tem moof+mdat)
pub fn is_fmp4_segment(data: &[u8]) -> bool {
    let boxes = parse_mp4_boxes(data);
    boxes.iter().any(|b| b.box_type == "moof")
}

/// Detecta se um segmento é TS (começa com sync byte 0x47)
pub fn is_ts_segment(data: &[u8]) -> bool {
    if data.len() < 188 {
        return false;
    }
    // TS packet sync byte
    data[0] == 0x47 || (data.len() >= 376 && data[188] == 0x47)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mp4_boxes() {
        // Minimal ftyp box: size=20, type="ftyp", brand="isom", version=0
        let mut data = Vec::new();
        // ftyp box (20 bytes)
        data.extend_from_slice(&20u32.to_be_bytes()); // size
        data.extend_from_slice(b"ftyp"); // type
        data.extend_from_slice(b"isom"); // brand
        data.extend_from_slice(&0u32.to_be_bytes()); // version
        data.extend_from_slice(b"isom"); // compatible brand

        // free box (8 bytes)
        data.extend_from_slice(&8u32.to_be_bytes());
        data.extend_from_slice(b"free");

        let boxes = parse_mp4_boxes(&data);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].box_type, "ftyp");
        assert_eq!(boxes[0].size, 20);
        assert_eq!(boxes[1].box_type, "free");
        assert_eq!(boxes[1].size, 8);
    }

    #[test]
    fn test_is_ts_segment() {
        let mut ts_data = vec![0u8; 376];
        ts_data[0] = 0x47; // sync byte
        ts_data[188] = 0x47; // segundo pacote
        assert!(is_ts_segment(&ts_data));

        let non_ts = vec![0u8; 376];
        assert!(!is_ts_segment(&non_ts));
    }

    #[test]
    fn test_is_valid_fmp4() {
        let mut data = Vec::new();
        // ftyp
        data.extend_from_slice(&20u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(b"isom");
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"isom");
        // moov (minimal)
        data.extend_from_slice(&8u32.to_be_bytes());
        data.extend_from_slice(b"moov");

        assert!(is_valid_fmp4_init(&data));
    }
}
