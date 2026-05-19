#[derive(Debug)]
pub enum RedirectOp {
    Output(String),
    Append(String),
    Input(String),
    Stderr(String),
    StderrToStdout,
}

#[derive(Debug)]
pub struct SimpleCommand {
    pub name: String,
    pub args: Vec<String>,
    pub redirects: Vec<RedirectOp>,
}

#[derive(Debug)]
pub enum AstNode {
    Simple(SimpleCommand),
    Pipeline(Vec<AstNode>),
    And(Box<AstNode>, Box<AstNode>),
    Or(Box<AstNode>, Box<AstNode>),
    Seq(Box<AstNode>, Box<AstNode>),
}

pub fn parse(input: &str) -> Result<AstNode, String> {
    let tokens = tokenize(input)?;
    parse_sequence(&tokens)
}

pub(crate) fn tokenize(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                current.push(ch);
            }
        } else if in_double {
            if ch == '"' {
                in_double = false;
            } else if ch == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else {
                current.push(ch);
            }
        } else if ch == '\'' {
            in_single = true;
        } else if ch == '"' {
            in_double = true;
        } else if ch == '#' && current.is_empty() {
            break;
        } else if ch == '|' {
            if chars.peek() == Some(&'|') {
                chars.next();
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push("||".into());
            } else {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push("|".into());
            }
        } else if ch == ';' {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            tokens.push(";".into());
        } else if ch == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push("&&".into());
            } else {
                current.push(ch);
            }
        } else if ch == '>' {
            let fd = if current == "1" || current == "2" {
                let fd = current.clone();
                current.clear();
                Some(fd)
            } else {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                None
            };

            if chars.peek() == Some(&'>') {
                chars.next();
                let tok = match fd {
                    Some(fd) => format!("{fd}>>"),
                    None => ">>".to_string(),
                };
                tokens.push(tok);
            } else if chars.peek() == Some(&'&') {
                chars.next();
                // Read the target file descriptor after >& (e.g., 2>&1 → target=1).
                let target = chars
                    .next_if(|c| c.is_ascii_digit())
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "1".to_string());
                let tok = match fd {
                    Some(fd) => format!("{fd}>&{target}"),
                    None => format!("2>&{target}"),
                };
                tokens.push(tok);
            } else {
                let tok = match fd {
                    Some(fd) => format!("{fd}>"),
                    None => ">".to_string(),
                };
                tokens.push(tok);
            }
        } else if ch == '<' {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            tokens.push("<".into());
        } else if ch == ' ' || ch == '\t' {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

fn parse_sequence(tokens: &[String]) -> Result<AstNode, String> {
    let mut nodes = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == ";" {
            i += 1;
            continue;
        }
        let (node, consumed) = parse_and_or(tokens, i)?;
        nodes.push(node);
        i += consumed;
        if i < tokens.len() && tokens[i] == ";" {
            i += 1;
        }
    }

    if nodes.is_empty() {
        return Err("empty command".into());
    }

    let mut iter = nodes.into_iter();
    let mut result = iter.next().unwrap();
    for node in iter {
        result = AstNode::Seq(Box::new(result), Box::new(node));
    }
    Ok(result)
}

fn parse_and_or(tokens: &[String], start: usize) -> Result<(AstNode, usize), String> {
    let (left, mut consumed) = parse_pipeline(tokens, start)?;

    if start + consumed < tokens.len() {
        if tokens[start + consumed] == "&&" {
            let (right, right_consumed) = parse_and_or(tokens, start + consumed + 1)?;
            consumed += 1 + right_consumed;
            return Ok((AstNode::And(Box::new(left), Box::new(right)), consumed));
        } else if tokens[start + consumed] == "||" {
            let (right, right_consumed) = parse_and_or(tokens, start + consumed + 1)?;
            consumed += 1 + right_consumed;
            return Ok((AstNode::Or(Box::new(left), Box::new(right)), consumed));
        }
    }

    Ok((left, consumed))
}

fn parse_pipeline(tokens: &[String], start: usize) -> Result<(AstNode, usize), String> {
    let mut commands = Vec::new();
    let mut i = start;

    loop {
        let (cmd, consumed) = parse_simple(tokens, i)?;
        commands.push(cmd);
        i += consumed;

        if i < tokens.len() && tokens[i] == "|" {
            i += 1;
        } else {
            break;
        }
    }

    if commands.len() == 1 {
        Ok((commands.into_iter().next().unwrap(), i - start))
    } else {
        let pipeline = AstNode::Pipeline(commands);
        Ok((pipeline, i - start))
    }
}

fn parse_simple(tokens: &[String], start: usize) -> Result<(AstNode, usize), String> {
    if start >= tokens.len() {
        return Err("unexpected end".into());
    }

    let mut args = Vec::new();
    let mut redirects = Vec::new();
    let mut i = start;

    while i < tokens.len() {
        match tokens[i].as_str() {
            "|" | ";" | "&&" | "||" => break,
            ">" => {
                i += 1;
                if i < tokens.len() {
                    redirects.push(RedirectOp::Output(tokens[i].clone()));
                    i += 1;
                }
            }
            ">>" => {
                i += 1;
                if i < tokens.len() {
                    redirects.push(RedirectOp::Append(tokens[i].clone()));
                    i += 1;
                }
            }
            "<" => {
                i += 1;
                if i < tokens.len() {
                    redirects.push(RedirectOp::Input(tokens[i].clone()));
                    i += 1;
                }
            }
            "1>" | "2>" => {
                i += 1;
                if i < tokens.len() {
                    let op = if tokens[i - 1] == "2>" {
                        RedirectOp::Stderr(tokens[i].clone())
                    } else {
                        RedirectOp::Output(tokens[i].clone())
                    };
                    redirects.push(op);
                    i += 1;
                }
            }
            "1>>" | "2>>" => {
                i += 1;
                if i < tokens.len() {
                    let op = if tokens[i - 1] == "2>>" {
                        RedirectOp::Stderr(tokens[i].clone())
                    } else {
                        RedirectOp::Append(tokens[i].clone())
                    };
                    redirects.push(op);
                    i += 1;
                }
            }
            tok if tok.contains(">&") => {
                // Handle [n]>&[m] redirect — any source fd to any target fd.
                // In our simplified shell, 2>&1 (stderr→stdout) is the primary
                // use case, but we accept the general form to avoid silently
                // dropping unrecognized redirect tokens.
                redirects.push(RedirectOp::StderrToStdout);
                i += 1;
            }
            _ => {
                args.push(tokens[i].clone());
                i += 1;
            }
        }
    }

    if args.is_empty() {
        return Err("empty command".into());
    }

    Ok((
        AstNode::Simple(SimpleCommand {
            name: args.remove(0),
            args,
            redirects,
        }),
        i - start,
    ))
}
