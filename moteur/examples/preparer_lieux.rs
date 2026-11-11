//! Refait `modeles/lieux` à partir des fichiers de GeoNames et du CLDR déjà téléchargés.
//! cargo run --release -p furet-moteur --example preparer_lieux -- <dossier des sources> <dossier de sortie>
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (source, sortie) = (args.next().expect("sources"), args.next().expect("sortie"));
    furet_moteur::lieux::preparer(std::path::Path::new(&source), std::path::Path::new(&sortie))?;
    println!("ok");
    Ok(())
}
