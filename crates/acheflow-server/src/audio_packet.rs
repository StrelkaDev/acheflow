//! Enhanced RTMP v2 audio demultiplexing, before independent FFmpeg decoders.
//! https://github.com/veovera/enhanced-rtmp/blob/main/docs/enhanced/enhanced-rtmp-v2.md

#[derive(Debug)]
pub struct AudioPacket {
    pub track_id: u8,
    pub codec: &'static str,
    pub ended: bool,
    pub data: Vec<u8>,
}

pub fn split_audio(data: &[u8]) -> Result<Vec<AudioPacket>, String> {
    if data.is_empty() {
        return Ok(vec![]);
    }
    if data.len() > 2 * 1024 * 1024 {
        return Err("Audio message exceeds 2 MiB".into());
    }
    if data[0] >> 4 != 9 {
        let codec = match data[0] >> 4 {
            0 => "pcm",
            1 => "adpcm_swf",
            2 | 14 => "mp3",
            3 => "pcm_s16le",
            4..=6 => "nellymoser",
            7 => "pcm_alaw",
            8 => "pcm_mulaw",
            10 => "aac",
            11 => "speex",
            _ => return Err("Unsupported legacy RTMP audio codec".into()),
        };
        if data.len() < 2 {
            return Err("Truncated legacy audio packet".into());
        }
        return Ok(vec![AudioPacket {
            track_id: 0,
            codec,
            ended: false,
            data: data.to_vec(),
        }]);
    }
    let mut cursor = 1;
    let mut kind = data[0] & 15;
    let multitrack = kind == 5;
    let mode = if multitrack {
        let types = *take(data, &mut cursor, 1)?.first().unwrap();
        kind = types & 15;
        types >> 4
    } else {
        0
    };
    if mode > 2 || !matches!(kind, 0 | 1 | 2 | 4) {
        return Err("Unsupported Enhanced RTMP audio packet/extension".into());
    }
    let mut fourcc = take(data, &mut cursor, 4)?;
    let mut result = vec![];
    let mut ids = std::collections::HashSet::new();
    loop {
        let codec = match fourcc {
            b"mp4a" => "aac",
            b"Opus" => "opus",
            b".mp3" => "mp3",
            b"fLaC" => "flac",
            b"ac-3" => "ac3",
            b"ec-3" => "eac3",
            _ => return Err("Unsupported Enhanced RTMP audio FourCC".into()),
        };
        let track_id = if multitrack {
            take(data, &mut cursor, 1)?[0]
        } else {
            0
        };
        if !ids.insert(track_id) {
            return Err("Duplicate audio track in RTMP message".into());
        }
        if result.len() == 8 {
            return Err("At most 8 audio tracks per RTMP message".into());
        }
        let size = if mode == 0 {
            data.len() - cursor
        } else {
            let bytes = take(data, &mut cursor, 3)?;
            (usize::from(bytes[0]) << 16) | (usize::from(bytes[1]) << 8) | usize::from(bytes[2])
        };
        let body = take(data, &mut cursor, size)?;
        let mut normalized = Vec::with_capacity(5 + body.len());
        normalized.push(0x90 | kind);
        normalized.extend_from_slice(fourcc);
        normalized.extend_from_slice(body);
        result.push(AudioPacket {
            track_id,
            codec,
            ended: kind == 2,
            data: normalized,
        });
        if cursor == data.len() {
            break;
        }
        if mode == 0 {
            return Err("Trailing single-track audio data".into());
        }
        if mode == 2 {
            fourcc = take(data, &mut cursor, 4)?;
        }
    }
    Ok(result)
}

fn take<'a>(data: &'a [u8], cursor: &mut usize, size: usize) -> Result<&'a [u8], String> {
    let end = cursor
        .checked_add(size)
        .ok_or("Audio packet size overflow")?;
    let bytes = data
        .get(*cursor..end)
        .ok_or("Truncated Enhanced RTMP audio packet")?;
    *cursor = end;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_and_batched_tracks_keep_ids_and_payloads() {
        let one = split_audio(&[0x95, 0x01, b'm', b'p', b'4', b'a', 7, 11, 22]).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].track_id, 7);
        assert_eq!(one[0].codec, "aac");
        assert_eq!(one[0].data, [0x91, b'm', b'p', b'4', b'a', 11, 22]);
        let many = split_audio(&[
            0x95, 0x11, b'm', b'p', b'4', b'a', 0, 0, 0, 2, 11, 22, 9, 0, 0, 1, 33,
        ])
        .unwrap();
        assert_eq!(many.len(), 2);
        assert_eq!(many[1].track_id, 9);
        assert_eq!(*many[1].data.last().unwrap(), 33);
        let different = split_audio(&[
            0x95, 0x21, b'm', b'p', b'4', b'a', 0, 0, 0, 1, 11, b'O', b'p', b'u', b's', 5, 0, 0, 1,
            22,
        ])
        .unwrap();
        assert_eq!(different[1].track_id, 5);
        assert_eq!(different[1].codec, "opus");
    }

    #[test]
    fn legacy_audio_is_track_zero_and_silence_is_empty() {
        let packet = split_audio(&[0xaf, 0, 0x11, 0x90]).unwrap();
        assert_eq!(packet[0].track_id, 0);
        assert_eq!(packet[0].data, [0xaf, 0, 0x11, 0x90]);
        assert!(split_audio(&[]).unwrap().is_empty());
    }

    #[test]
    fn malformed_or_unsupported_packets_are_not_silently_truncated() {
        for data in [
            vec![0x95],
            vec![0x95, 0x31],
            vec![0x90, b'?', b'?', b'?', b'?'],
            vec![0x97],
            vec![0x95, 0x11, b'm', b'p', b'4', b'a', 0, 0xff, 0xff, 0xff],
            vec![
                0x95, 0x11, b'm', b'p', b'4', b'a', 0, 0, 0, 1, 8, 0, 0, 0, 1, 9,
            ],
        ] {
            assert!(split_audio(&data).is_err(), "{data:?}");
        }
    }
}
