//! Sceglie il provider crittografico di rustls.
//!
//! Nel progetto sono compilati sia `ring` sia `aws-lc-rs` (uno per ogni libreria di rete):
//! senza una scelta esplicita, rustls va in panico alla prima connessione TLS.

pub fn install_crypto_provider() {
    // Errore = un provider è già installato: va bene così.
    let _ = rustls::crypto::ring::default_provider().install_default();
}
