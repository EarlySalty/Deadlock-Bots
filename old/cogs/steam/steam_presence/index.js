'use strict';
// Die alte Node-Brücke darf keinen zweiten Login oder Token-Dateischreiber
// mehr starten. Die vollständige Umsetzung liegt im Rust-Steam-Core.
console.error('Die alte Steam-Brücke ist stillgelegt. Bitte steam-core aus Deadlock-Steam-Bot verwenden. Kontozugänge liegen ausschließlich verschlüsselt in der zentralen Datenbank.');
process.exitCode = 1;
