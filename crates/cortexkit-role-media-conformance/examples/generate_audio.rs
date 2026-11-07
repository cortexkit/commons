use cortexkit_role_media_conformance::fixtures::{sha256, AudioFixture};

fn main() -> std::io::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/media-v1");
    std::fs::create_dir_all(root.join("audio"))?;
    let mut sums = String::new();
    for (name, fixture) in [
        ("silence", AudioFixture::Silence),
        ("tone", AudioFixture::Tone),
        ("noise", AudioFixture::Noise),
    ] {
        let wav = fixture.wav();
        let path = format!("audio/{name}.wav");
        sums.push_str(&format!("{}  {path}\n", sha256(&wav)));
        std::fs::write(root.join(path), wav)?;
    }
    std::fs::write(root.join("SHA256SUMS"), sums)
}
