//! Generate derived Frontier response candidates from caller-supplied original software data.
use std::path::PathBuf;
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        eprintln!(
            "Usage: antitone-frontier-bake <ColorData.dat> <ez171autoc-nega.dat> <fresh-output-directory>"
        );
        std::process::exit(2);
    }
    let paths: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    if let Err(error) =
        antitone::looks::frontier::authoring::generate(&paths[0], &paths[1], &paths[2])
    {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
