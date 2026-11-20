// Icône et nom de l'application dans l'exe : visibles dans l'Explorateur Windows,
// la barre des tâches et le Gestionnaire des tâches (qui affiche « Vigie »).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("FileDescription", "Vigie")
            // Commandes sans fenêtre comprises par cet exe (lu par Ultimate avant d'appeler --etat-dev…).
            // Ajouter ici toute nouvelle commande sans fenêtre.
            .set("VigieCli", "avant-jeu apres-jeu etat-dev couper-dev memoire-jeu")
            .set("ProductName", "Vigie")
            .set("InternalName", "vigie")
            .set("OriginalFilename", "Vigie.exe")
            .set("CompanyName", "Vigie")
            .set("LegalCopyright", "");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icône non intégrée : {e}");
        }
    }
}
