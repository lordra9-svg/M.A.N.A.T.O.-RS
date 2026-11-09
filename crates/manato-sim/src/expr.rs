//! Expression parser for M.U.G.E.N trigger and controller expressions.
//!
//! The grammar is intentionally permissive. Unknown bare identifiers evaluate to
//! strings, which makes attribute comparisons such as `statetype = A` and
//! `hitdefattr = SC, NA` work without special-casing the lexer.

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i32),
    Float(f32),
    Str(String),
    Ident(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Redirect(String, Box<Expr>),
    Range {
        lo: Box<Expr>,
        hi: Box<Expr>,
        lo_incl: bool,
        hi_incl: bool,
    },
    List(Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i32),
    Float(f32),
    Str(String),
    Bool(bool),
}

impl Default for Value {
    fn default() -> Self {
        Self::Int(0)
    }
}

impl Value {
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Int(v) => *v as f32,
            Self::Float(v) => *v,
            Self::Bool(v) => *v as i32 as f32,
            Self::Str(s) => s.trim().parse().unwrap_or(0.0),
        }
    }

    pub fn as_i32(&self) -> i32 {
        match self {
            Self::Int(v) => *v,
            Self::Float(v) => *v as i32,
            Self::Bool(v) => *v as i32,
            Self::Str(s) => s.trim().parse().unwrap_or(0),
        }
    }

    pub fn as_str(&self) -> String {
        match self {
            Self::Int(v) => v.to_string(),
            Self::Float(v) => v.to_string(),
            Self::Bool(v) => (*v as i32).to_string(),
            Self::Str(s) => s.clone(),
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Self::Int(v) => *v != 0,
            Self::Float(v) => *v != 0.0,
            Self::Bool(v) => *v,
            Self::Str(s) => !s.is_empty() && s != "0",
        }
    }

    pub fn is_int(&self) -> bool {
        matches!(self, Self::Int(_) | Self::Bool(_))
    }
}

pub fn parse(input: &str) -> Result<Expr, String> {
    let tokens = lex(input)?;
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.parse_expr()?;
    if p.pos != p.tokens.len() {
        return Err(format!("unexpected token {:?}", p.tokens[p.pos]));
    }
    Ok(expr)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f32, bool),
    Str(String),
    Ident(String),
    Sym(&'static str),
}

fn lex(input: &str) -> Result<Vec<Tok>, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == ';' {
            break;
        }
        if c == '"' {
            let start = i + 1;
            i += 1;
            while i < bytes.len() && bytes[i] as char != '"' {
                i += 1;
            }
            if i >= bytes.len() {
                return Err("unterminated string".into());
            }
            out.push(Tok::Str(input[start..i].to_string()));
            i += 1;
            continue;
        }
        if c.is_ascii_digit()
            || (c == '.' && i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_digit())
        {
            let start = i;
            let mut is_float = false;
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch.is_ascii_digit() {
                    i += 1;
                } else if ch == '.' && !is_float {
                    is_float = true;
                    i += 1;
                } else {
                    break;
                }
            }
            let text = &input[start..i];
            let value = text.parse::<f32>().map_err(|e| e.to_string())?;
            out.push(Tok::Num(value, !is_float));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Tok::Ident(input[start..i].to_string()));
            continue;
        }
        let rest = &input[i..];
        let (sym, len) = if rest.starts_with("&&") {
            ("&&", 2)
        } else if rest.starts_with("||") {
            ("||", 2)
        } else if rest.starts_with("!=") {
            ("!=", 2)
        } else if rest.starts_with("<=") {
            ("<=", 2)
        } else if rest.starts_with(">=") {
            (">=", 2)
        } else if rest.starts_with("**") {
            ("**", 2)
        } else {
            match c {
                '+' => ("+", 1),
                '-' => ("-", 1),
                '*' => ("*", 1),
                '/' => ("/", 1),
                '%' => ("%", 1),
                '=' => ("=", 1),
                '<' => ("<", 1),
                '>' => (">", 1),
                '!' => ("!", 1),
                '&' => ("&", 1),
                '|' => ("|", 1),
                '^' => ("^", 1),
                '~' => ("~", 1),
                '(' => ("(", 1),
                ')' => (")", 1),
                '[' => ("[", 1),
                ']' => ("]", 1),
                ',' => (",", 1),
                _ => return Err(format!("unexpected character {c:?}")),
            }
        };
        out.push(Tok::Sym(sym));
        i += len;
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn eat_sym(&mut self, sym: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(s)) if *s == sym) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_sym(&mut self, sym: &str) -> Result<(), String> {
        if self.eat_sym(sym) {
            Ok(())
        } else {
            Err(format!("expected {sym:?}, found {:?}", self.peek()))
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.eat_sym("||") {
            let right = self.parse_and()?;
            left = Expr::Binary(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bitwise()?;
        while self.eat_sym("&&") {
            let right = self.parse_bitwise()?;
            left = Expr::Binary(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_bitwise(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_cmp()?;
        loop {
            let op = if self.eat_sym("&") {
                Some(BinOp::BitAnd)
            } else if self.eat_sym("|") {
                Some(BinOp::BitOr)
            } else if self.eat_sym("^") {
                Some(BinOp::BitXor)
            } else {
                None
            };
            let Some(op) = op else { break };
            let right = self.parse_cmp()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_cmp(&mut self) -> Result<Expr, String> {
        let left = self.parse_add()?;
        let op = if self.eat_sym("=") {
            Some(BinOp::Eq)
        } else if self.eat_sym("!=") {
            Some(BinOp::Ne)
        } else if self.eat_sym("<=") {
            Some(BinOp::Le)
        } else if self.eat_sym(">=") {
            Some(BinOp::Ge)
        } else if self.eat_sym("<") {
            Some(BinOp::Lt)
        } else if self.eat_sym(">") {
            Some(BinOp::Gt)
        } else {
            None
        };
        let Some(op) = op else { return Ok(left) };
        let right = self.parse_add()?;
        // Attribute lists: `hitdefattr = SC, NA, SA, HA`.
        if matches!(self.peek(), Some(Tok::Sym(","))) {
            let mut list = vec![right];
            while self.eat_sym(",") {
                list.push(self.parse_add()?);
            }
            return Ok(Expr::Binary(op, Box::new(left), Box::new(Expr::List(list))));
        }
        Ok(Expr::Binary(op, Box::new(left), Box::new(right)))
    }

    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mul()?;
        loop {
            let op = if self.eat_sym("+") {
                Some(BinOp::Add)
            } else if self.eat_sym("-") {
                Some(BinOp::Sub)
            } else {
                None
            };
            let Some(op) = op else { break };
            let right = self.parse_mul()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = if self.eat_sym("*") {
                Some(BinOp::Mul)
            } else if self.eat_sym("/") {
                Some(BinOp::Div)
            } else if self.eat_sym("%") {
                Some(BinOp::Mod)
            } else if self.eat_sym("**") {
                Some(BinOp::Pow)
            } else {
                None
            };
            let Some(op) = op else { break };
            let right = self.parse_unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.eat_sym("!") {
            return Ok(Expr::Unary(UnOp::Not, Box::new(self.parse_unary()?)));
        }
        if self.eat_sym("-") {
            return Ok(Expr::Unary(UnOp::Neg, Box::new(self.parse_unary()?)));
        }
        if self.eat_sym("+") {
            return self.parse_unary();
        }
        if self.eat_sym("~") {
            return Ok(Expr::Unary(UnOp::BitNot, Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        let Some(tok) = self.peek().cloned() else {
            return Err("unexpected end of expression".into());
        };
        match tok {
            Tok::Num(v, is_int) => {
                self.pos += 1;
                if is_int {
                    Ok(Expr::Int(v as i32))
                } else {
                    Ok(Expr::Float(v))
                }
            }
            Tok::Str(s) => {
                self.pos += 1;
                Ok(Expr::Str(s))
            }
            Tok::Sym("(") => {
                self.pos += 1;
                let e = self.parse_expr()?;
                self.expect_sym(")")?;
                Ok(e)
            }
            Tok::Sym("[") => {
                self.pos += 1;
                let lo = self.parse_add()?;
                self.expect_sym(",")?;
                let hi = self.parse_add()?;
                let lo_incl = true;
                let hi_incl = if self.eat_sym("]") {
                    true
                } else {
                    self.expect_sym(")")?;
                    false
                };
                Ok(Expr::Range {
                    lo: Box::new(lo),
                    hi: Box::new(hi),
                    lo_incl,
                    hi_incl,
                })
            }
            Tok::Ident(name) => {
                self.pos += 1;
                // Redirect prefixes: `p2, life`, `enemy(0), stateno`.
                if matches!(self.peek(), Some(Tok::Sym(","))) && is_redirect(&name) {
                    self.pos += 1;
                    let inner = self.parse_expr()?;
                    return Ok(Expr::Redirect(name, Box::new(inner)));
                }
                if is_redirect(&name) && matches!(self.peek(), Some(Tok::Sym("("))) {
                    let save = self.pos;
                    self.pos += 1;
                    if let Some(Tok::Num(_, _)) = self.peek() {
                        let Tok::Num(n, _) = self.tokens[self.pos].clone() else {
                            unreachable!()
                        };
                        self.pos += 1;
                        if self.eat_sym(")") && self.eat_sym(",") {
                            let inner = self.parse_expr()?;
                            return Ok(Expr::Redirect(
                                format!("{name}({})", n as i32),
                                Box::new(inner),
                            ));
                        }
                    }
                    self.pos = save;
                }
                if self.eat_sym("(") {
                    let mut args = Vec::new();
                    if !self.eat_sym(")") {
                        loop {
                            args.push(self.parse_expr()?);
                            if self.eat_sym(",") {
                                continue;
                            }
                            self.expect_sym(")")?;
                            break;
                        }
                    }
                    return Ok(Expr::Call(name, args));
                }
                // Two-token triggers such as `vel x`, `pos y`, `p2bodydist X`.
                if needs_axis_arg(&name) {
                    if let Some(Tok::Ident(axis)) = self.peek().cloned() {
                        self.pos += 1;
                        return Ok(Expr::Call(name, vec![Expr::Str(axis)]));
                    }
                }
                Ok(Expr::Ident(name))
            }
            other => Err(format!("unexpected token {other:?}")),
        }
    }
}

fn is_redirect(name: &str) -> bool {
    match name.to_ascii_lowercase().as_str() {
        "p1" | "p2" | "p3" | "p4" | "enemy" | "enemynear" | "target" | "root" | "parent"
        | "self" | "playerid" | "playerindex" | "helper" | "helperindex" | "stateowner"
        | "partner" => true,
        _ => {
            name.to_ascii_lowercase().starts_with("enemy")
                || name.to_ascii_lowercase().starts_with("target")
        }
    }
}

fn needs_axis_arg(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "vel" | "pos" | "p2dist" | "p2bodydist" | "p2screendist" | "screenpos" | "screenvel"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_triggers() {
        for src in [
            "Time = 0",
            "Anim != 0 && Anim != 5",
            "const(velocity.walk.fwd.x)",
            "ifelse(sysvar(1)=0, const(velocity.jump.neu.x), 3)",
            "stateno != [3050,3100)",
            "hitdefattr = SC, NA, SA, HA",
            "p2bodydist X < 3",
            "vel x > 0",
            "GetHitVar(animtype) = [3,5]",
            "command != \"holddown\"",
            "(stateno = 200) && time > 5",
            "!alive",
            "power >= 1000",
        ] {
            parse(src).unwrap_or_else(|e| panic!("failed to parse {src:?}: {e}"));
        }
    }

    #[test]
    fn parses_redirect() {
        let e = parse("p2, life").unwrap();
        assert!(matches!(e, Expr::Redirect(_, _)));
    }
}
