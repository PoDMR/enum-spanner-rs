mod glushkov;
mod parse;

use super::automaton::Automaton;
use super::mapping;

pub fn compile(regex: &str) -> Automaton {
    let regex = reformat(regex);
    let hir = parse::Hir::from_regex(&regex);
    glushkov::LocalLang::from_hir(hir, 0).into_automaton()
}

pub fn is_match(regex: &str, text: &str) -> bool {
    let automaton = compile(&regex);
    let matches = compile_matches(&automaton, text);

    // TODO: investigate this weird borrow checker behaviour
    let ret = matches.iter().next().is_some();
    ret
}

pub fn compile_matches<'a>(automaton: &'a Automaton, text: &'a str) -> mapping::IndexedDag {
    mapping::IndexedDag::compile(automaton.clone(), text.to_string())
}

/// Reformat the regex to get a regex matching the whole regex in a group called *match*.
/// The new regex will allow any prefix or suffix to be matched before the old regex, except if
/// the input regex contains anchors at its begining or end.
fn reformat(regex: &str) -> String {
    let regex = match regex.as_bytes().first() {
        Some(c) if *c == b'^' => format!("(?P<match>{}", &regex[1..]),
        _ => format!(r"(.|\s)*(?P<match>{}", regex),
    };

    let regex = match regex.as_bytes().last() {
        Some(c) if *c == b'$' => format!("{})", &regex[..regex.len() - 1]),
        _ => format!(r"{})(.|\s)*", regex),
    };

    regex
}

#[cfg(test)]
mod tests;
