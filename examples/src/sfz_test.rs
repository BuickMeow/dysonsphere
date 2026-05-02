use std::path::Path;

fn main() {
    let sfz_paths = [
        "/Users/jieneng/Documents/Soundfonts/Starry Studio Grand 2.5/Presets/D_Simplicity/Studio Grand - Simplicity (2 Sec Release).sfz",
        "/Users/jieneng/Documents/Soundfonts/Starry Studio Grand 2.5/Presets/D_Simplicity/Studio Grand - Simplicity (5 Sec Release, 70% Reverb).sfz",
    ];

    for sfz_path in &sfz_paths {
        eprintln!("=== Testing: {sfz_path}");
        match dysonphere_soundfont::sfz::load(Path::new(sfz_path), 44100) {
            Ok(sf) => {
                eprintln!("  OK: {} presets, {} regions, {} sample buffers",
                    sf.presets.len(),
                    sf.presets.first().map(|p| p.regions.len()).unwrap_or(0),
                    sf.sample_buffers.len());
                // List some presets
                for preset in &sf.presets {
                    eprintln!("    Preset: bank={} prog={} name={} regions={}",
                        preset.bank, preset.program, preset.name, preset.regions.len());
                }
                // Try voice_params for note 60
                let params = sf.voice_params(0, 0, 60, 100, 44100);
                eprintln!("  Voice params for note 60 vel=100: {}", params.len());
                if !params.is_empty() {
                    eprintln!("    volume={:.3} speed_mult={:.3} sample_len={}",
                        params[0].volume, params[0].speed_mult, params[0].sample.len());
                }
            }
            Err(e) => {
                eprintln!("  FAILED: {}", e);
            }
        }
    }
}
