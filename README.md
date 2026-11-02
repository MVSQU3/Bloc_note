# Bloc_note

Bloc-notes pour Windows écrit en Rust (egui/eframe).

Fonctions : onglets + restauration de session, ouvrir/enregistrer (UTF-8, UTF-8 BOM, UTF-16, ANSI, CRLF/LF/CR),
annuler/rétablir, rechercher/remplacer (casse, mot entier, regex), atteindre la ligne, zoom (Ctrl+molette),
retour à la ligne, barre d'état, thème clair/sombre, Heure/Date (F5), glisser-déposer,
et listes (puces, tirets, numérotées, a/A, i/I, tâches) avec continuation sur Entrée et retrait Tab/Maj+Tab
(Ctrl+Maj+8 puces, Ctrl+Maj+7 numérotée).

## Compiler sous Windows
    cargo build --release
Exécutable : `target/release/bloc_note.exe`
