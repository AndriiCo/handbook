//! One file out of an Apple disk image (`.dmg`), in memory, for the
//! wizard's brand Fetch: Apple ships its Sign in with Apple logo only as
//! a `.dmg` (`Logo-Sign-in-with-Apple.dmg`), and `hdiutil` exists only on
//! macOS, so the wizard reads the image itself. Nothing is mounted or
//! unpacked to disk.
//!
//! Two layers, both bounds-checked (the download is untrusted):
//! - **UDIF**: the 512-byte `koly` trailer points at an XML property
//!   list whose `blkx` entries describe each partition as chunks
//!   (`mish` tables). The `Apple_HFS` partition is rebuilt from its
//!   chunks: raw, zero-fill and zlib (`UDZO`, what Apple's logo image
//!   uses). Other compressions (bzip2, lzfse, ADC) are refused with a
//!   pointer to opening the image on a Mac.
//! - **HFS+**: the volume header names the catalog file; its B-tree
//!   leaf records are scanned for the file whose name equals `want`,
//!   and its data fork (up to the eight extents a catalog record
//!   holds) is read back.

use std::io::Read as _;

use base64::Engine as _;

/// The rebuilt partition never grows past this (a decompression bomb
/// stops here); Apple's logo image is about 15 MB.
const MAX_IMAGE: u64 = 64 * 1024 * 1024;

fn slice(b: &[u8], at: usize, len: usize) -> Result<&[u8], String> {
    at.checked_add(len)
        .and_then(|end| b.get(at..end))
        .ok_or_else(|| "the disk image is truncated or malformed".to_string())
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, String> {
    Ok(u16::from_be_bytes(slice(b, at, 2)?.try_into().unwrap()))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_be_bytes(slice(b, at, 4)?.try_into().unwrap()))
}

fn u64_at(b: &[u8], at: usize) -> Result<u64, String> {
    Ok(u64::from_be_bytes(slice(b, at, 8)?.try_into().unwrap()))
}

fn usize_of(v: u64) -> Result<usize, String> {
    usize::try_from(v).map_err(|_| "a disk image offset is out of range".to_string())
}

/// True when `bytes` ends with a UDIF `koly` trailer.
pub fn is_dmg(bytes: &[u8]) -> bool {
    bytes.len() >= 512 && bytes[bytes.len() - 512..].starts_with(b"koly")
}

/// The file called `want` from the image's HFS+ partition, at most
/// `max` bytes.
pub fn file(bytes: &[u8], want: &str, max: usize) -> Result<Vec<u8>, String> {
    let partition = hfs_partition(bytes)?;
    hfs_file(&partition, want, max)
}

/// The `(Name, Data)` of every `blkx` entry in the trailer's plist.
fn blkx_entries(plist: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let start = plist
        .find("<key>blkx</key>")
        .ok_or("the disk image has no partition table (blkx)")?;
    let rest = &plist[start..];
    let end = rest.find("</array>").unwrap_or(rest.len());
    let mut out = Vec::new();
    for dict in rest[..end].split("<dict>").skip(1) {
        let value = |key: &str, tag: &str| -> Option<String> {
            let at = dict.find(&format!("<key>{key}</key>"))?;
            let after = &dict[at..];
            let open = after.find(&format!("<{tag}>"))? + tag.len() + 2;
            let close = after[open..].find(&format!("</{tag}>"))?;
            Some(after[open..open + close].to_string())
        };
        let (Some(name), Some(data)) = (value("Name", "string"), value("Data", "data")) else {
            continue;
        };
        let compact: String = data.chars().filter(|c| !c.is_whitespace()).collect();
        let data = base64::engine::general_purpose::STANDARD
            .decode(compact)
            .map_err(|_| "the disk image's partition table is not base64")?;
        out.push((name, data));
    }
    Ok(out)
}

/// The `Apple_HFS` (or `Apple_HFSX`) partition, rebuilt from its chunks.
fn hfs_partition(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if !is_dmg(bytes) {
        return Err("not an Apple disk image (no koly trailer)".into());
    }
    let koly = &bytes[bytes.len() - 512..];
    let data_fork = usize_of(u64_at(koly, 0x18)?)?;
    let xml_at = usize_of(u64_at(koly, 0xD8)?)?;
    let xml_len = usize_of(u64_at(koly, 0xE0)?)?;
    let plist = std::str::from_utf8(slice(bytes, xml_at, xml_len)?)
        .map_err(|_| "the disk image's property list is not UTF-8")?;
    let (_, mish) = blkx_entries(plist)?
        .into_iter()
        .find(|(name, _)| name.contains("Apple_HFS"))
        .ok_or("the disk image has no HFS+ partition")?;
    if !mish.starts_with(b"mish") {
        return Err("the disk image's block table is malformed".into());
    }
    let sectors = u64_at(&mish, 16)?;
    if sectors.saturating_mul(512) > MAX_IMAGE {
        return Err("the disk image's partition is too large".into());
    }
    let mut image = vec![0u8; usize_of(sectors * 512)?];
    let chunks = u32_at(&mish, 200)? as usize;
    for i in 0..chunks {
        let at = 204 + i * 40;
        let kind = u32_at(&mish, at)?;
        let start = usize_of(u64_at(&mish, at + 8)?.saturating_mul(512))?;
        let count = usize_of(u64_at(&mish, at + 16)?.saturating_mul(512))?;
        let src_at = data_fork
            .checked_add(usize_of(u64_at(&mish, at + 24)?)?)
            .ok_or("a chunk offset is out of range")?;
        let src = slice(bytes, src_at, usize_of(u64_at(&mish, at + 32)?)?)?;
        let dest = image
            .get_mut(start..start.saturating_add(count))
            .ok_or("a chunk lies outside its partition")?;
        match kind {
            // zero-fill / ignored blocks: already zero
            0x0000_0000 | 0x0000_0002 => {}
            0x0000_0001 => {
                let n = src.len().min(dest.len());
                dest[..n].copy_from_slice(&src[..n]);
            }
            0x8000_0005 => {
                let mut out = Vec::with_capacity(dest.len());
                flate2::read::ZlibDecoder::new(src)
                    .take(dest.len() as u64)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("a zlib chunk does not inflate: {e}"))?;
                dest[..out.len()].copy_from_slice(&out);
            }
            0x7FFF_FFFE | 0xFFFF_FFFF => {}
            other => {
                return Err(format!(
                    "the disk image uses compression {other:#x} this tool does not read; \
                     open it on a Mac and drop the file by hand"
                ));
            }
        }
    }
    Ok(image)
}

/// A fork's bytes from its 80-byte `HFSPlusForkData`.
fn fork(image: &[u8], fork: &[u8], block: usize, max: usize) -> Result<Vec<u8>, String> {
    let size = usize_of(u64_at(fork, 0)?)?;
    if size > max {
        return Err(format!("the file is {size} bytes, over the {max}-byte cap"));
    }
    let mut out = Vec::with_capacity(size);
    for i in 0..8 {
        let start = u32_at(fork, 16 + i * 8)? as usize;
        let count = u32_at(fork, 20 + i * 8)? as usize;
        if count == 0 || out.len() >= size {
            break;
        }
        let at = start
            .checked_mul(block)
            .ok_or("an extent is out of range")?;
        let len = count
            .checked_mul(block)
            .ok_or("an extent is out of range")?;
        out.extend_from_slice(slice(image, at, len.min(size - out.len()))?);
    }
    if out.len() < size {
        return Err("the file continues past its first eight extents (not read)".into());
    }
    Ok(out)
}

/// The file called `want` in an HFS+ volume image.
fn hfs_file(image: &[u8], want: &str, max: usize) -> Result<Vec<u8>, String> {
    let header = slice(image, 1024, 512)?;
    if !(header.starts_with(b"H+") || header.starts_with(b"HX")) {
        return Err("the partition is not HFS+".into());
    }
    let block = u32_at(header, 40)? as usize;
    if !(512..=65536).contains(&block) {
        return Err("the HFS+ block size is implausible".into());
    }
    let catalog = fork(image, slice(header, 272, 80)?, block, MAX_IMAGE as usize)?;
    let node_size = u16_at(&catalog, 32)? as usize;
    if node_size < 512 {
        return Err("the HFS+ catalog node size is implausible".into());
    }
    let mut found = Vec::new();
    for node in catalog.chunks_exact(node_size) {
        // Leaf nodes (kind -1) hold the catalog records.
        if node[8] != 0xFF {
            continue;
        }
        for i in 0..u16_at(node, 10)? as usize {
            let slot = node_size
                .checked_sub(2 * (i + 1))
                .ok_or("an HFS+ catalog node claims too many records")?;
            let at = u16_at(node, slot)? as usize;
            let key_len = u16_at(node, at)? as usize;
            let name_len = u16_at(node, at + 6)? as usize;
            let (pairs, _) = slice(node, at + 8, name_len * 2)?.as_chunks::<2>();
            let units: Vec<u16> = pairs.iter().map(|c| u16::from_be_bytes(*c)).collect();
            let mut record = at + 2 + key_len;
            record += record % 2;
            // Record type 2: a file.
            if u16_at(node, record)? != 2 || String::from_utf16_lossy(&units) != want {
                continue;
            }
            found.push(fork(image, slice(node, record + 88, 80)?, block, max)?);
        }
    }
    match found.len() {
        1 => Ok(found.pop().unwrap()),
        0 => Err(format!("no {want:?} in the disk image")),
        n => Err(format!("{want:?} names {n} files in the disk image")),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write as _;

    const BLOCK: usize = 4096;
    const NODE: usize = 4096;

    /// A minimal HFS+ volume: header, a two-node catalog (header node +
    /// one leaf) and one block per file.
    pub(crate) fn hfs(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut image = vec![0u8; BLOCK * (3 + files.len())];
        image[1024..1026].copy_from_slice(b"H+");
        image[1024 + 40..1024 + 44].copy_from_slice(&(BLOCK as u32).to_be_bytes());
        // Catalog fork: two nodes in blocks 1-2.
        let cat = 1024 + 272;
        image[cat..cat + 8].copy_from_slice(&((2 * NODE) as u64).to_be_bytes());
        image[cat + 16..cat + 20].copy_from_slice(&1u32.to_be_bytes());
        image[cat + 20..cat + 24].copy_from_slice(&2u32.to_be_bytes());
        let header_node = BLOCK;
        image[header_node + 32..header_node + 34].copy_from_slice(&(NODE as u16).to_be_bytes());
        let leaf = BLOCK + NODE;
        image[leaf + 8] = 0xFF;
        image[leaf + 10..leaf + 12].copy_from_slice(&(files.len() as u16).to_be_bytes());
        let mut at = 14;
        for (i, (name, body)) in files.iter().enumerate() {
            let units: Vec<u16> = name.encode_utf16().collect();
            let key_len = 6 + 2 * units.len();
            let rec = &mut image[leaf..leaf + NODE];
            rec[at..at + 2].copy_from_slice(&(key_len as u16).to_be_bytes());
            rec[at + 6..at + 8].copy_from_slice(&(units.len() as u16).to_be_bytes());
            for (j, u) in units.iter().enumerate() {
                rec[at + 8 + 2 * j..at + 10 + 2 * j].copy_from_slice(&u.to_be_bytes());
            }
            let mut data = at + 2 + key_len;
            data += data % 2;
            rec[data..data + 2].copy_from_slice(&2u16.to_be_bytes());
            let fork = data + 88;
            rec[fork..fork + 8].copy_from_slice(&(body.len() as u64).to_be_bytes());
            let file_block = (3 + i) as u32;
            rec[fork + 16..fork + 20].copy_from_slice(&file_block.to_be_bytes());
            rec[fork + 20..fork + 24].copy_from_slice(&1u32.to_be_bytes());
            let slot = NODE - 2 * (i + 1);
            rec[slot..slot + 2].copy_from_slice(&(at as u16).to_be_bytes());
            at = data + 248;
            let file_at = (3 + i) * BLOCK;
            image[file_at..file_at + body.len()].copy_from_slice(body);
        }
        image
    }

    /// Wrap a partition in a UDIF image: one zlib chunk, the plist and
    /// the koly trailer.
    pub(crate) fn udif(partition: &[u8]) -> Vec<u8> {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(partition).unwrap();
        let data = z.finish().unwrap();
        let mut mish = vec![0u8; 204 + 2 * 40];
        mish[..4].copy_from_slice(b"mish");
        mish[16..24].copy_from_slice(&((partition.len() / 512) as u64).to_be_bytes());
        mish[200..204].copy_from_slice(&2u32.to_be_bytes());
        mish[204..208].copy_from_slice(&0x8000_0005u32.to_be_bytes());
        mish[204 + 16..204 + 24].copy_from_slice(&((partition.len() / 512) as u64).to_be_bytes());
        mish[204 + 32..204 + 40].copy_from_slice(&(data.len() as u64).to_be_bytes());
        mish[244..248].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        let b64 = base64::engine::general_purpose::STANDARD.encode(&mish);
        let plist = format!(
            "<?xml version=\"1.0\"?><plist><dict><key>resource-fork</key><dict>\
             <key>blkx</key><array><dict><key>Data</key><data>AAAA</data>\
             <key>Name</key><string>Driver Descriptor Map (DDM : 0)</string></dict>\
             <dict><key>Data</key><data>\n{b64}\n</data>\
             <key>Name</key><string>disk image (Apple_HFS : 2)</string></dict>\
             </array></dict></dict></plist>"
        );
        let mut out = data.clone();
        let xml_at = out.len() as u64;
        out.extend_from_slice(plist.as_bytes());
        let mut koly = vec![0u8; 512];
        koly[..4].copy_from_slice(b"koly");
        koly[0xD8..0xE0].copy_from_slice(&xml_at.to_be_bytes());
        koly[0xE0..0xE8].copy_from_slice(&(plist.len() as u64).to_be_bytes());
        out.extend_from_slice(&koly);
        out
    }

    #[test]
    fn reads_one_named_file_out_of_a_zlib_disk_image() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>"#;
        let dmg = udif(&hfs(&[
            ("Logo - Black.svg", svg),
            ("Logo - White.svg", b"<svg/>"),
        ]));
        assert!(is_dmg(&dmg));
        assert_eq!(file(&dmg, "Logo - Black.svg", 4096).unwrap(), svg);
        assert_eq!(file(&dmg, "Logo - White.svg", 4096).unwrap(), b"<svg/>");
        let missing = file(&dmg, "Logo - Red.svg", 4096).unwrap_err();
        assert!(missing.contains("no \"Logo - Red.svg\""), "{missing}");
        let capped = file(&dmg, "Logo - Black.svg", 10).unwrap_err();
        assert!(capped.contains("over the 10-byte cap"), "{capped}");
    }

    #[test]
    fn malformed_images_are_refused_not_panicking() {
        assert!(!is_dmg(b"PK\x03\x04 a zip"));
        assert!(file(b"short", "x", 10).is_err());
        let good = udif(&hfs(&[("a.svg", b"<svg/>")]));
        assert_eq!(file(&good, "a.svg", 4096).unwrap(), b"<svg/>");
        let koly_at = good.len() - 512;
        let mutate = |f: &dyn Fn(&mut Vec<u8>)| {
            let mut bad = good.clone();
            f(&mut bad);
            bad
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            // the plist offset points past the end
            (
                "xml offset",
                mutate(&|b| {
                    b[koly_at + 0xD8..koly_at + 0xE0].copy_from_slice(&u64::MAX.to_be_bytes())
                }),
            ),
            // the compressed partition is garbage
            (
                "zlib",
                mutate(&|b| b[..64].iter_mut().for_each(|x| *x ^= 0x5A)),
            ),
            // half the image is gone (offsets point past the end)
            ("truncated", {
                let mut b = good[..koly_at / 2].to_vec();
                b.extend_from_slice(&good[koly_at..]);
                b
            }),
        ];
        for (what, bad) in cases {
            assert!(file(&bad, "a.svg", 4096).is_err(), "{what}");
        }
        // A partition that is not HFS+.
        let not_hfs = udif(&vec![0u8; BLOCK * 3]);
        assert!(
            file(&not_hfs, "a.svg", 4096)
                .unwrap_err()
                .contains("not HFS+")
        );
    }

    /// Apple's real image, when `APPLE_DMG` names a downloaded copy of
    /// Logo-Sign-in-with-Apple.dmg (not committed: Apple's licence).
    #[test]
    #[ignore]
    fn reads_apples_logo_image() {
        let path = std::env::var("APPLE_DMG").expect("APPLE_DMG=<path to the .dmg>");
        let dmg = std::fs::read(path).unwrap();
        let svg = file(
            &dmg,
            "Logo - SIWA - Left-aligned - Black - Medium.svg",
            1 << 20,
        )
        .unwrap();
        assert!(String::from_utf8(svg).unwrap().contains("<svg"));
    }
}
