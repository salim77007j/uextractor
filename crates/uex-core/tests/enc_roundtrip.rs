use uex_core::crypto::AesKey;
use uex_core::pak::{PakReader, PakWriter, Version};

#[test]
fn enc_roundtrip_debug() {
    let key = AesKey::parse("11223344556677889900AABBCCDDEEFF00112233445566778899AABBCCDDEEFF").unwrap();
    let cursor = std::io::Cursor::new(Vec::new());
    let mut w = PakWriter::new(cursor, Version::V11, "../../../T/", Some(key.clone()), vec![], true);
    w.write_file("A/Test.uasset", b"hello world test data 123456").unwrap();
    w.write_file("B/Test.uexp", b"second entry payload").unwrap();
    let buf = w.finalize().unwrap().into_inner();
    std::fs::write("/tmp/enc_debug.pak", &buf).unwrap();
    let f = std::fs::File::open("/tmp/enc_debug.pak").unwrap();
    let r = PakReader::new(std::io::BufReader::new(f), Some(key));
    match r {
        Ok(reader) => {
            println!("files: {:?}", reader.files());
            assert!(reader.files().len() == 2);
        }
        Err(e) => panic!("reader failed: {e:?}"),
    }
}
