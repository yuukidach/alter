//! A small, deterministic calculator for launcher expressions.
//!
//! This intentionally does not evaluate code or invoke a shell.  It accepts
//! arithmetic expressions made up of numbers, parentheses and the operators
//! `+`, `-`, `*`, `/`, `%` and `^` (with `×`/`÷` accepted as aliases).

pub fn evaluate(raw: &str) -> Option<String> {
    let mut expression = raw.trim();
    let explicit = expression.starts_with('=');
    if explicit {
        expression = expression[1..].trim_start();
    }
    if expression.is_empty() || expression.chars().count() > 256 {
        return None;
    }

    let normalized = expression
        .chars()
        .map(|character| match character {
            '×' => '*',
            '÷' => '/',
            character => character,
        })
        .collect::<String>();
    let mut parser = Parser::new(&normalized);
    let value = parser.expression()?;
    parser.skip_whitespace();
    if parser.position != parser.characters.len()
        || (!explicit && !parser.saw_operator)
        || !value.is_finite()
    {
        return None;
    }
    Some(format_number(value))
}

fn format_number(value: f64) -> String {
    // Avoid displaying a surprising negative zero after a subtraction or a
    // very small rounded result.
    let value = if value.abs() < 1e-12 { 0.0 } else { value };
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_991.0 {
        return format!("{value:.0}");
    }

    let mut formatted = format!("{value:.12}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    formatted
}

struct Parser {
    characters: Vec<char>,
    position: usize,
    saw_operator: bool,
}

impl Parser {
    fn new(expression: &str) -> Self {
        Self {
            characters: expression.chars().collect(),
            position: 0,
            saw_operator: false,
        }
    }

    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            self.skip_whitespace();
            let operator = match self.peek() {
                Some('+') | Some('-') => self.next().unwrap(),
                _ => break,
            };
            self.saw_operator = true;
            let right = self.term()?;
            value = if operator == '+' {
                value + right
            } else {
                value - right
            };
        }
        Some(value)
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.power()?;
        loop {
            self.skip_whitespace();
            let operator = match self.peek() {
                Some('*') | Some('/') | Some('%') => self.next().unwrap(),
                _ => break,
            };
            self.saw_operator = true;
            let right = self.power()?;
            value = match operator {
                '*' => value * right,
                '/' if right != 0.0 => value / right,
                '%' if right != 0.0 => value % right,
                _ => return None,
            };
        }
        Some(value)
    }

    fn power(&mut self) -> Option<f64> {
        let left = self.unary()?;
        self.skip_whitespace();
        if self.peek() != Some('^') {
            return Some(left);
        }
        self.next();
        self.saw_operator = true;
        // Recursive parsing makes exponentiation right associative: 2^3^2
        // is interpreted as 2^(3^2).
        let right = self.power()?;
        let value = left.powf(right);
        value.is_finite().then_some(value)
    }

    fn unary(&mut self) -> Option<f64> {
        self.skip_whitespace();
        match self.peek() {
            Some('+') => {
                self.next();
                self.saw_operator = true;
                self.unary()
            }
            Some('-') => {
                self.next();
                self.saw_operator = true;
                Some(-self.unary()?)
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Option<f64> {
        self.skip_whitespace();
        if self.peek() == Some('(') {
            self.next();
            let value = self.expression()?;
            self.skip_whitespace();
            if self.next() != Some(')') {
                return None;
            }
            return Some(value);
        }

        let start = self.position;
        let mut saw_digit = false;
        let mut saw_dot = false;
        while let Some(character) = self.peek() {
            if character.is_ascii_digit() {
                saw_digit = true;
                self.next();
            } else if character == '.' && !saw_dot {
                saw_dot = true;
                self.next();
            } else {
                break;
            }
        }
        if !saw_digit {
            return None;
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.next();
            if matches!(self.peek(), Some('+' | '-')) {
                self.next();
            }
            let exponent_start = self.position;
            while self
                .peek()
                .is_some_and(|character| character.is_ascii_digit())
            {
                self.next();
            }
            if exponent_start == self.position {
                return None;
            }
        }
        self.characters[start..self.position]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.characters.get(self.position).copied()
    }

    fn next(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.position += 1;
        Some(character)
    }
}

#[cfg(test)]
mod tests {
    use super::evaluate;

    #[test]
    fn evaluates_common_expressions() {
        assert_eq!(evaluate("2 + 3 * 4"), Some("14".to_owned()));
        assert_eq!(evaluate("(10 - 4) / 3"), Some("2".to_owned()));
        assert_eq!(evaluate("2^3^2"), Some("512".to_owned()));
        assert_eq!(evaluate("10 % 3"), Some("1".to_owned()));
    }

    #[test]
    fn rejects_unsafe_or_invalid_input() {
        assert_eq!(evaluate("hello"), None);
        assert_eq!(evaluate("1 / 0"), None);
        assert_eq!(evaluate("2 + (3"), None);
        assert_eq!(evaluate("=42"), Some("42".to_owned()));
    }
}
