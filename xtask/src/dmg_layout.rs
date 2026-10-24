//! `cargo xtask dmg-layout DIR`: write `DIR/.DS_Store`, the Finder window of the macOS disk image
//! (packaging/macos/package.sh). The window shows `.background.tiff` from the root of the image (the
//! drag arrow), SolidCraft.app on the left and Applications on the right, without toolbars.
//!
//! Finder keeps this layout in a `.DS_Store` file: a B-tree of records in a buddy-allocated file
//! (format notes: https://metacpan.org/dist/Mac-Finder-DSStore/view/DSStoreFormat.pod). The
//! background is an alias record that names the image by its path on the volume, so the image can be
//! built with `hdiutil makehybrid` and never attached while it is written.

use std::path::Path;

/// The volume name `hdiutil makehybrid -hfs-volume-name` gives the image (package.sh).
const VOLUME: &str = "SolidCraft";
/// The background in the root of the image: `tiffutil -cathidpicheck` of
/// assets/platforms/macos/dmg-background.png and its @2x (package.sh).
const BACKGROUND: &str = ".background.tiff";
/// Window position and size in points. The background is 640x430; the window size includes the
/// title bar, so it is a little taller. With Finder's path bar, status bar or tab bar turned on the
/// bottom of the background is cut off, but the icons and labels still fit.
const WINDOW: (u32, u32, u32, u32) = (200, 140, 640, 456);
const ICON_SIZE: f64 = 112.0;
const TEXT_SIZE: f64 = 13.0;
/// Icon centres, either side of the arrow in the middle of the background. Finder draws the labels
/// in black, so the background has a light pill under each label (at y = 277); move them together.
const ICONS: [(&str, u32, u32); 2] = [("SolidCraft.app", 122, 205), ("Applications", 522, 205)];

pub fn run(args: &[&str]) -> Result<(), String> {
    let [dir] = args else { return Err("usage: cargo xtask dmg-layout <folder that becomes the disk image>".into()) };
    let path = Path::new(dir).join(".DS_Store");
    std::fs::write(&path, ds_store()?).map_err(|e| format!("{}: {e}", path.display()))
}

/// The `.DS_Store` file for the image's root folder.
fn ds_store() -> Result<Vec<u8>, String> {
    use Plist::*;
    let (x, y, w, h) = WINDOW;
    let window = Dict(vec![
        ("ContainerShowSidebar", Bool(false)),
        ("PreviewPaneVisibility", Bool(false)),
        ("ShowPathbar", Bool(false)),
        ("ShowSidebar", Bool(false)),
        ("ShowStatusBar", Bool(false)),
        ("ShowTabView", Bool(false)),
        ("ShowToolbar", Bool(false)),
        ("SidebarWidth", Int(180)),
        ("WindowBounds", Str(format!("{{{{{x}, {y}}}, {{{w}, {h}}}}}"))),
    ]);
    let icon_view = Dict(vec![
        ("arrangeBy", Str("none".into())),
        ("backgroundColorBlue", Real(1.0)),
        ("backgroundColorGreen", Real(1.0)),
        ("backgroundColorRed", Real(1.0)),
        ("backgroundImageAlias", Data(alias(VOLUME, BACKGROUND))),
        // 2: a picture (0 is the default white, 1 a colour).
        ("backgroundType", Int(2)),
        ("gridOffsetX", Real(0.0)),
        ("gridOffsetY", Real(0.0)),
        ("gridSpacing", Real(100.0)),
        ("iconSize", Real(ICON_SIZE)),
        ("labelOnBottom", Bool(true)),
        ("scrollPositionX", Real(0.0)),
        ("scrollPositionY", Real(0.0)),
        ("showIconPreview", Bool(false)),
        ("showItemInfo", Bool(false)),
        ("textSize", Real(TEXT_SIZE)),
        ("viewOptionsVersion", Int(1)),
    ]);
    let mut records = vec![
        (".".to_string(), *b"bwsp", Value::Blob(bplist(&window)?)),
        (".".to_string(), *b"icvl", Value::Type(*b"icnv")),
        (".".to_string(), *b"icvp", Value::Blob(bplist(&icon_view)?)),
        (".".to_string(), *b"vSrn", Value::Long(1)),
    ];
    for (name, x, y) in ICONS {
        // Iloc: the icon centre, then 6 bytes Finder always writes.
        let iloc = [&x.to_be_bytes()[..], &y.to_be_bytes(), &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0]].concat();
        records.push((name.to_string(), *b"Iloc", Value::Blob(iloc)));
    }
    Ok(buddy_file(&records))
}

enum Value {
    Long(u32),
    Type([u8; 4]),
    Blob(Vec<u8>),
}

/// Lay out a `.DS_Store` with all records in one leaf node. The buddy allocator's address space
/// starts 4 bytes into the file; its blocks are the header (0x0, 32 bytes), the DSDB header
/// (0x20, 32), the allocator's own table (0x800, 2 KiB) and the leaf (0x2000, 8 KiB), the layout a
/// new file from Finder has. Every other power-of-two block up to 2 GiB is on a free list.
fn buddy_file(records: &[(String, [u8; 4], Value)]) -> Vec<u8> {
    const TABLE: usize = 0x800;
    const DSDB: usize = 0x20;
    const LEAF: usize = 0x2000;
    let mut sorted: Vec<_> = records.iter().collect();
    // Finder looks records up by binary search: file names case-insensitively, then the code.
    sorted.sort_by_key(|(name, code, _)| (name.to_lowercase(), *code));
    let mut leaf = Vec::new();
    put32(&mut leaf, 0); // no children
    put32(&mut leaf, sorted.len() as u32);
    for (name, code, value) in sorted {
        let units: Vec<u16> = name.encode_utf16().collect();
        put32(&mut leaf, units.len() as u32);
        units.iter().for_each(|u| leaf.extend(u.to_be_bytes()));
        leaf.extend(code);
        match value {
            Value::Long(v) => {
                leaf.extend(b"long");
                put32(&mut leaf, *v);
            }
            Value::Type(t) => {
                leaf.extend(b"type");
                leaf.extend(t);
            }
            Value::Blob(b) => {
                leaf.extend(b"blob");
                put32(&mut leaf, b.len() as u32);
                leaf.extend(b);
            }
        }
    }
    assert!(leaf.len() <= 0x2000, "the layout outgrew one B-tree node");

    let mut dsdb = Vec::new();
    // Root node (block 2), levels below it, records, nodes, page size.
    for v in [2, 0, records.len() as u32, 1, 0x1000] {
        put32(&mut dsdb, v);
    }

    let mut table = Vec::new();
    put32(&mut table, 3);
    put32(&mut table, 0);
    // Block addresses ORed with log2 of the size, padded to a multiple of 256 entries.
    for block in [TABLE | 11, DSDB | 5, LEAF | 13] {
        put32(&mut table, block as u32);
    }
    table.resize(table.len() + 253 * 4, 0);
    put32(&mut table, 1);
    table.push(4);
    table.extend(b"DSDB");
    put32(&mut table, 1);
    for log2 in 0..32 {
        if (6..=30).contains(&log2) && log2 != 11 && log2 != 13 {
            put32(&mut table, 1);
            put32(&mut table, 1 << log2);
        } else {
            put32(&mut table, 0);
        }
    }

    let mut space = vec![0; 0x4000];
    let mut header = b"Bud1".to_vec();
    for v in [TABLE, TABLE, TABLE] {
        put32(&mut header, v as u32);
    }
    // Unknown; the same bytes as in Finder's own files.
    header.extend([0, 0, 0x10, 0x0c, 0, 0, 0, 0x87, 0, 0, 0x20, 0x0b, 0, 0, 0, 0]);
    for (at, block) in [(0, header), (DSDB, dsdb), (TABLE, table), (LEAF, leaf)] {
        space[at..at + block.len()].copy_from_slice(&block);
    }
    [&[0, 0, 0, 1][..], &space].concat()
}

/// A version 2 alias record for `file` in the root of the volume `volume`. The volume date and
/// the catalog node ids are left 0: they change with every image, so Finder finds the file by its
/// path on the mounted volume instead.
fn alias(volume: &str, file: &str) -> Vec<u8> {
    let pascal = |out: &mut Vec<u8>, s: &str, size: usize| {
        out.push(s.len() as u8);
        out.extend(s.as_bytes());
        out.resize(out.len() + size - 1 - s.len(), 0);
    };
    let mut a = vec![0; 4]; // application info
    a.extend([0, 0, 0, 2]); // record length (below), version 2
    a.extend([0, 0]); // kind: file
    pascal(&mut a, volume, 28);
    put32(&mut a, 0); // volume creation date
    a.extend(b"H+");
    a.extend([0, 0]); // fixed disk
    put32(&mut a, 2); // parent: the root folder
    pascal(&mut a, file, 64);
    put32(&mut a, 0); // file id
    put32(&mut a, 0); // file creation date
    a.extend([0; 8]); // creator and type codes
    a.extend([0xff, 0xff, 0xff, 0xff]); // levels from and to: none
    a.extend([0; 16]); // volume attributes, file system id, reserved
    let utf16 = |s: &str| {
        let units: Vec<u16> = s.encode_utf16().collect();
        [&(units.len() as u16).to_be_bytes()[..], &units.iter().flat_map(|u| u.to_be_bytes()).collect::<Vec<_>>()].concat()
    };
    let extras: [(u16, Vec<u8>); 6] = [
        (0, volume.into()),                        // parent folder name
        (2, format!("{volume}:{file}").into()),    // Carbon path
        (14, utf16(file)),                         // file name
        (15, utf16(volume)),                       // volume name
        (18, format!("/{file}").into()),           // path on the volume
        (19, format!("/Volumes/{volume}").into()), // where the volume is mounted
    ];
    for (tag, data) in extras {
        a.extend(tag.to_be_bytes());
        a.extend((data.len() as u16).to_be_bytes());
        a.extend(&data);
        if data.len() % 2 == 1 {
            a.push(0);
        }
    }
    a.extend([0xff, 0xff, 0, 0]); // end
    let len = a.len() as u16;
    a[4..6].copy_from_slice(&len.to_be_bytes());
    a
}

enum Plist {
    Bool(bool),
    Int(u64),
    Real(f64),
    Str(String),
    Data(Vec<u8>),
    Dict(Vec<(&'static str, Plist)>),
}

/// A binary property list (`bplist00`), the form Finder keeps its view settings in.
fn bplist(top: &Plist) -> Result<Vec<u8>, String> {
    fn marker(kind: u8, len: usize, out: &mut Vec<u8>) {
        if len < 15 {
            out.push(kind | len as u8);
        } else {
            out.push(kind | 0xf);
            int(len as u64, out);
        }
    }
    fn int(v: u64, out: &mut Vec<u8>) {
        match v {
            0..=0xff => out.extend([0x10, v as u8]),
            0x100..=0xffff => out.extend([&[0x11][..], &(v as u16).to_be_bytes()].concat()),
            _ => out.extend([&[0x13][..], &v.to_be_bytes()].concat()),
        }
    }
    fn add(p: &Plist, objects: &mut Vec<Vec<u8>>) -> Result<usize, String> {
        let index = objects.len();
        objects.push(Vec::new());
        let mut out = Vec::new();
        match p {
            Plist::Bool(b) => out.push(if *b { 0x09 } else { 0x08 }),
            Plist::Int(v) => int(*v, &mut out),
            Plist::Real(v) => out.extend([&[0x23][..], &v.to_be_bytes()].concat()),
            Plist::Str(s) => {
                if !s.is_ascii() {
                    return Err(format!("plist string {s:?}: only ASCII is written"));
                }
                marker(0x50, s.len(), &mut out);
                out.extend(s.as_bytes());
            }
            Plist::Data(d) => {
                marker(0x40, d.len(), &mut out);
                out.extend(d);
            }
            Plist::Dict(entries) => {
                let keys = entries.iter().map(|(k, _)| add(&Plist::Str(k.to_string()), objects)).collect::<Result<Vec<_>, _>>()?;
                let values = entries.iter().map(|(_, v)| add(v, objects)).collect::<Result<Vec<_>, _>>()?;
                marker(0xd0, entries.len(), &mut out);
                for r in keys.into_iter().chain(values) {
                    out.push(u8::try_from(r).map_err(|_| "plist has more than 256 objects")?);
                }
            }
        }
        objects[index] = out;
        Ok(index)
    }
    let mut objects = Vec::new();
    add(top, &mut objects)?;
    let mut out = b"bplist00".to_vec();
    let mut offsets = Vec::new();
    for o in &objects {
        offsets.push(out.len());
        out.extend(o);
    }
    let table = out.len();
    let offset_size = if table <= 0xffff { 2 } else { 4 };
    for o in offsets {
        out.extend(&(o as u32).to_be_bytes()[4 - offset_size..]);
    }
    out.extend([0; 6]);
    out.extend([offset_size as u8, 1]); // offset and object reference sizes
    for v in [objects.len(), 0, table] {
        out.extend((v as u64).to_be_bytes());
    }
    Ok(out)
}

fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend(v.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_names_the_background_by_its_path() {
        let a = alias(VOLUME, BACKGROUND);
        assert_eq!(u16::from_be_bytes([a[4], a[5]]) as usize, a.len());
        assert!(a.ends_with(&[0xff, 0xff, 0, 0]));
        let has = |s: &str| a.windows(s.len()).any(|w| w == s.as_bytes());
        assert!(has("/.background.tiff") && has("/Volumes/SolidCraft") && has("SolidCraft:.background.tiff"));
    }

    #[test]
    fn ds_store_layout() {
        let f = ds_store().unwrap();
        assert_eq!(f.len(), 4 + 0x4000);
        assert_eq!(&f[4..8], b"Bud1");
        // The leaf: no children, 6 records, the first the window settings of the folder itself.
        let leaf = &f[4 + 0x2000..];
        assert_eq!(&leaf[..8], &[0, 0, 0, 0, 0, 0, 0, 6]);
        assert_eq!(&leaf[8..20], &[0, 0, 0, 1, 0, b'.', b'b', b'w', b's', b'p', b'b', b'l']);
        let iloc = [b"Iloc".as_slice(), b"blob", &[0, 0, 0, 16, 0, 0, 0, 122, 0, 0, 0, 205]].concat();
        assert!(leaf.windows(iloc.len()).any(|w| w == iloc));
    }

    /// macOS reads the property lists back as written.
    #[cfg(target_os = "macos")]
    #[test]
    fn plutil_reads_the_plists() {
        use Plist::*;
        let p = Dict(vec![
            ("WindowBounds", Str("{{200, 140}, {640, 456}}".into())),
            ("a", Bool(true)),
            ("b", Int(180)),
            ("c", Real(13.0)),
            ("d", Data(vec![7; 300])),
        ]);
        let path = std::env::temp_dir().join(format!("xtask-dmg-layout-{}.plist", std::process::id()));
        std::fs::write(&path, bplist(&p).unwrap()).unwrap();
        let out = std::process::Command::new("plutil").args(["-convert", "xml1", "-o", "-"]).arg(&path).output().unwrap();
        std::fs::remove_file(&path).ok();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let xml = String::from_utf8(out.stdout).unwrap();
        for want in ["<string>{{200, 140}, {640, 456}}</string>", "<key>a</key>\n\t<true/>", "<integer>180</integer>", "<real>13</real>", "<data>"] {
            assert!(xml.contains(want), "{want} missing from\n{xml}");
        }
    }
}
