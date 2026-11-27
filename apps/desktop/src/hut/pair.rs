//! Reading the code `emberhut-server pair --json` prints.

use serde::Deserialize;

/// The line `pair --json` prints (its other fields are not needed here).
#[derive(Deserialize)]
struct Printed {
    /// The code, as shown.
    code: String,
}

/// The pairing code in `stdout`, the output of `emberhut-server pair --json`: the `code` of
/// its JSON line (any other line, such as a log line, is passed over), else the code of a
/// `Pairing code: ABCD-EFGH …` line, as a server without `--json` prints it. Only a code of
/// the server's shape counts: eight capitals and digits, dashed after four. `None` without one.
#[must_use]
pub(crate) fn code_in(stdout: &str) -> Option<String> {
    let from_json = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Printed>(line.trim()).ok())
        .map(|printed| printed.code);
    let from_words = stdout.lines().filter_map(|line| {
        line.trim()
            .strip_prefix("Pairing code:")
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_owned)
    });
    from_json
        .chain(from_words)
        .map(|code| code.trim().to_ascii_uppercase())
        .find(|code| is_code(code))
}

/// Whether `code` reads `XXXX-XXXX`, in capitals and digits.
fn is_code(code: &str) -> bool {
    let Some((first, second)) = code.split_once('-') else {
        return false;
    };
    [first, second]
        .iter()
        .all(|half| half.len() == 4 && half.bytes().all(|b| b.is_ascii_alphanumeric()))
}
