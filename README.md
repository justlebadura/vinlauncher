# vinlauncher

Fork personal de [OneLauncher/OneClient](https://github.com/Polyfrost/OneLauncher) (Polyfrost), bajo GPL-3.0.

Cambios respecto al upstream:

- Rebranding completo a **vinlauncher**.
- Catálogo curado de bundles de Polyfrost desactivado: los modpacks se añaden por instancia importando archivos `.mrpack` (Modrinth).
- Sin orientación a Hypixel.
- Features sociales de Poly+ eliminadas (chat, parties/lobby, wardrobe).
- Auto-updater desactivado.

## Compilar

Requiere Rust 1.97+ (se probó con 1.100).

```bash
cargo build --release -p oneclient_app
./target/release/oneclient_app
```

## Créditos

Todo el mérito del código base es de [Polyfrost](https://polyfrost.org). Este fork respeta la licencia GPL-3.0 del proyecto original.
