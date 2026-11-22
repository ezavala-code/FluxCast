mod client;

use std::{env, io, os::unix::process::CommandExt, path::PathBuf, process::Command};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => {
            if let Some(runtime) = env::var_os("XDG_RUNTIME_DIR") {
                if client::toggle(PathBuf::from(runtime).as_path()).is_ok() {
                    return Ok(());
                }
            }
            let desktop = env::current_exe()?.with_file_name("fluxcast-desktop");
            Err(Command::new(desktop).exec().into())
        }
        [arg] if arg == "toggle" => {
            let runtime =
                PathBuf::from(env::var_os("XDG_RUNTIME_DIR").ok_or("Falta XDG_RUNTIME_DIR")?);
            client::toggle(&runtime).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                    "FluxCast no está en ejecución. Inícialo primero con 'fluxcast'.".into()
                }
                _ => format!("No se pudo alternar FluxCast: {e}").into(),
            })
        }
        [arg] if arg == "--help" || arg == "-h" => {
            println!(
                "fluxcast         Iniciar el launcher o alternar si ya está activo\nfluxcast toggle  Mostrar u ocultar la ventana (<5 ms)"
            );
            Ok(())
        }
        _ => Err("Uso: fluxcast [toggle]".into()),
    }
}
