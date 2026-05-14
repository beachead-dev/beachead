// beachead-askpass: GIT_ASKPASS credential helper binary.
// Reads credentials from the OS keyring and prints them to stdout for git.
//
// Git invokes this binary with a prompt string as argv[1], e.g.:
//   "Username for 'https://github.com': "
//   "Password for 'https://github.com': "
//
// The binary determines whether a username or password is requested by
// checking if the prompt contains "password" (case-insensitive).
// It then reads the corresponding value from the OS keyring using the
// service name provided via the BEACHEAD_KEYRING_SERVICE environment variable.
//
// Security:
// - Credential values are never included in error output.
// - The credential string is zeroized from memory after printing.
// - Exits with code 1 on any failure, printing a diagnostic to stderr.

use keyring::Entry;
use std::env;
use std::io::Write;
use zeroize::Zeroize;

fn main() {
    if let Err(msg) = run() {
        eprintln!("beachead-askpass: {}", msg);
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    // Read the keyring service name from the environment.
    let service = env::var("BEACHEAD_KEYRING_SERVICE")
        .map_err(|_| "BEACHEAD_KEYRING_SERVICE environment variable not set".to_string())?;

    // Git passes the prompt as argv[1].
    let prompt = env::args().nth(1).unwrap_or_default();
    let is_password = prompt.to_lowercase().contains("password");

    // Construct the keyring entry key based on whether username or password is requested.
    let key = if is_password {
        format!("{}-secret", service)
    } else {
        format!("{}-username", service)
    };

    // Access the OS keyring.
    let entry = Entry::new(&key, "beachead")
        .map_err(|e| format!("failed to access keyring entry: {}", e))?;

    // Retrieve the credential value.
    let mut value = entry
        .get_password()
        .map_err(|_| "no credential found in keyring".to_string())?;

    // Print the value to stdout without a trailing newline.
    // Use write_all to ensure the full value is written even if it contains special chars.
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle
        .write_all(value.as_bytes())
        .map_err(|e| format!("failed to write to stdout: {}", e))?;
    handle
        .flush()
        .map_err(|e| format!("failed to flush stdout: {}", e))?;

    // Zeroize the credential from memory.
    value.zeroize();

    Ok(())
}
