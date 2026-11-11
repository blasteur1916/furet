//! Vérifie que tract sait lire les modèles de visages, et montre leurs entrées et sorties.
//! cargo run --release -p furet-moteur --example modeles_visages

use tract_onnx::prelude::*;

fn main() -> TractResult<()> {
    for (nom, forme) in [("face_detection_yunet_2023mar.onnx", [1, 3, 640, 640]), ("face_recognition_sface_2021dec.onnx", [1, 3, 112, 112])] {
        let chemin = format!(r"D:\furet\modeles\visages\{nom}");
        let modele = tract_onnx::onnx().model_for_path(&chemin)?;
        println!("== {nom}");
        for (i, e) in modele.input_outlets()?.iter().enumerate() {
            println!("  entrée {i} : {:?}", modele.outlet_fact(*e)?);
        }
        let plan = modele.with_input_fact(0, f32::fact(forme).into())?.into_optimized()?.into_runnable()?;
        let entree = Tensor::zero::<f32>(&forme)?;
        let t = std::time::Instant::now();
        let sorties = plan.run(tvec!(entree.into()))?;
        println!("  exécuté en {} ms", t.elapsed().as_millis());
        for (i, s) in sorties.iter().enumerate() {
            println!("  sortie {i} : {:?} {:?}", plan.model().output_fact(i).map(|f| f.shape.clone()).ok(), s.shape());
        }
    }
    Ok(())
}
