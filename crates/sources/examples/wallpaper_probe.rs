//! Prints what Status Quo finds as the current wallpaper on this machine and the hue it takes.
fn main() {
    match sq_sources::locate() {
        Ok(path) => {
            println!("wallpaper file: {}", path.display());
            match sq_sources::load_seed(&path) {
                Ok(seed) => {
                    println!("hue {:.0} chroma {:.2}", seed.hue, seed.chroma);
                    for dark in [true, false] {
                        println!(
                            "{} palette: {:?}",
                            if dark { "dark" } else { "light" },
                            sq_core::Palette::from_wallpaper(&seed, dark)
                        );
                    }
                }
                Err(error) => println!("cannot read it: {error}"),
            }
        }
        Err(error) => println!("not found: {error}"),
    }
}
