use std::borrow::Cow;

use nom::{
    branch::alt,
    bytes::complete::{tag, take},
    character::complete as ch,
    combinator::{all_consuming, map, map_parser, map_res, opt, peek, recognize},
    error::{Error as NomError, ErrorKind as NomErrorKind},
    multi::{length_data, many1_count},
    number::complete as num,
    sequence::{delimited, pair, preceded, terminated, tuple},
    Err as NomErr, IResult, Parser,
};

#[derive(Debug, PartialEq)]
pub enum SimulateArg<'s> {
    Acc(f32),
    Combo(u32),
    ClockRate(f32),
    N300(u32),
    N100(u32),
    N50(u32),
    Geki(u32),
    Katu(u32),
    Miss(u32),
    Mods(&'s str),
}

impl<'s> SimulateArg<'s> {
    pub fn parse(input: &'s str) -> Result<Self, ParseError> {
        let (rest, key_opt) = parse_key(input).map_err(|_| ParseError::Nom(input))?;

        match key_opt {
            None => parse_any(rest),
            Some("acc" | "a" | "accuracy") => parse_acc(rest).map(SimulateArg::Acc),
            Some("combo" | "c") => parse_combo(rest).map(SimulateArg::Combo),
            Some("clockrate" | "cr") => parse_clock_rate(rest).map(SimulateArg::ClockRate),
            Some("n300") => parse_n300(rest).map(SimulateArg::N300),
            Some("n100") => parse_n100(rest).map(SimulateArg::N100),
            Some("n50") => parse_n50(rest).map(SimulateArg::N50),
            Some("mods") => parse_mods(rest).map(SimulateArg::Mods),
            Some(key) => {
                let (sub_n, _) = opt::<_, _, NomError<_>, _>(ch::char('n'))(key)
                    .map_err(|_| ParseError::Nom(input))?;

                match sub_n {
                    "miss" | "m" | "misses" => parse_miss(rest).map(SimulateArg::Miss),
                    "geki" | "gekis" | "320" => parse_geki(rest).map(SimulateArg::Geki),
                    "katu" | "katus" | "200" => parse_katu(rest).map(SimulateArg::Katu),
                    _ => Err(ParseError::Unknown(key)),
                }
            }
        }
    }
}

fn parse_key(input: &str) -> IResult<&str, Option<&str>> {
    opt(terminated(ch::alphanumeric1, ch::char('=')))(input)
}

fn parse_any(input: &str) -> Result<SimulateArg, ParseError> {
    fn inner(input: &str) -> IResult<&str, SimulateArg> {
        enum ParseAny<'s> {
            Float(f32),
            Int(u32),
            Mods(&'s str),
        }

        let float = map(map_res(recognize_float, str::parse), ParseAny::Float);
        let int = map(ch::u32, ParseAny::Int);
        let mods = map(recognize_mods, ParseAny::Mods);
        let (rest, num) = alt((float, int, mods))(input)?;

        match num {
            ParseAny::Float(n) => {
                let acc = map(recognize_acc, |_| SimulateArg::Acc(n));
                let clock_rate = map(recognize_clock_rate, |_| SimulateArg::ClockRate(n));

                all_consuming(alt((acc, clock_rate)))(rest)
            }
            ParseAny::Int(n) => {
                let acc = map(recognize_acc, |_| SimulateArg::Acc(n as f32));
                let combo = map(recognize_combo, |_| SimulateArg::Combo(n));
                let clock_rate = map(ch::char('*'), |_| SimulateArg::ClockRate(n as f32));
                let n300 = map(recognize_n300, |_| SimulateArg::N300(n));
                let n100 = map(recognize_n100, |_| SimulateArg::N100(n));
                let n50 = map(recognize_n50, |_| SimulateArg::N50(n));
                let geki = map(recognize_geki, |_| SimulateArg::Geki(n));
                let katu = map(recognize_katu, |_| SimulateArg::Katu(n));
                let miss = map(recognize_miss, |_| SimulateArg::Miss(n));
                let options = (acc, combo, clock_rate, n300, n100, n50, geki, katu, miss);

                all_consuming(alt(options))(rest)
            }
            ParseAny::Mods(mods) if rest.is_empty() => Ok((rest, SimulateArg::Mods(mods))),
            ParseAny::Mods(_) => Err(NomErr::Error(NomError::new(input, NomErrorKind::Eof))),
        }
    }

    inner(input)
        .map(|(_, val)| val)
        .map_err(|_| ParseError::Nom(input))
}

fn parse_int<'i, F>(input: &'i str, suffix: F) -> IResult<&'i str, u32>
where
    F: Parser<&'i str, (), NomError<&'i str>>,
{
    all_consuming(terminated(ch::u32, opt(suffix)))(input)
}

fn parse_float<'i, F>(input: &'i str, suffix: F) -> IResult<&'i str, f32>
where
    F: Parser<&'i str, (), NomError<&'i str>>,
{
    all_consuming(terminated(num::float, opt(suffix)))(input)
}

macro_rules! parse_arg {
    ( $( $fn:ident -> $ty:ty: $parse:ident, $recognize:ident $( or $x:literal )?, $err:ident; )* ) => {
        $(
            fn $fn(input: &str) -> Result<$ty, ParseError> {
                let recognize = alt((
                    map($recognize, |_| ()),
                    $( map(ch::char($x), |_| ()) )?
                ));

                $parse(input, recognize)
                    .map(|(_, val)| val)
                    .map_err(|_| ParseError::$err)
            }
        )*
    };
}

parse_arg! {
    parse_acc -> f32: parse_float, recognize_acc, Acc;
    parse_combo -> u32: parse_int, recognize_combo, Combo;
    parse_clock_rate -> f32: parse_float, recognize_clock_rate, ClockRate;
    parse_n300 -> u32: parse_int, recognize_n300 or 'x', N300;
    parse_n100 -> u32: parse_int, recognize_n100 or 'x', N100;
    parse_n50 -> u32: parse_int, recognize_n50 or 'x', N50;
    parse_miss -> u32: parse_int, recognize_miss or 'x', Miss;
    parse_geki -> u32: parse_int, recognize_geki or 'x', Geki;
    parse_katu -> u32: parse_int, recognize_katu or 'x', Katu;
}

fn is_some<T>(opt: Option<T>) -> bool {
    opt.is_some()
}

fn parse_mods(input: &str) -> Result<&str, ParseError> {
    fn inner(input: &str) -> IResult<&str, &str> {
        let (rest, prefixed) = map(opt(ch::char('+')), is_some)(input)?;
        let (rest, mods) = parse_mods_raw(rest)?;
        let (rest, suffixed) = map(all_consuming(opt(ch::char('!'))), is_some)(rest)?;

        if prefixed || !suffixed {
            Ok((rest, mods))
        } else {
            Err(NomErr::Error(NomError::new(input, NomErrorKind::Verify)))
        }
    }

    inner(input)
        .map(|(_, val)| val)
        .map_err(|_| ParseError::Mods)
}

fn parse_mods_raw(input: &str) -> IResult<&str, &str> {
    let alpha1 = map_parser(take(1_usize), ch::alpha1);
    let alpha2 = map_parser(take(1_usize), ch::alpha1);
    let count = many1_count(pair(alpha1, alpha2)); // take an even amount of alphabetic chars

    length_data(map(peek(count), |n| n * 2))(input)
}

fn recognize_float(input: &str) -> IResult<&str, &str> {
    let comma = alt((ch::char('.'), ch::char(',')));

    recognize(tuple((ch::digit0, comma, ch::digit1)))(input)
}

fn recognize_acc(input: &str) -> IResult<&str, &str> {
    recognize(ch::char('%'))(input)
}

fn recognize_combo(input: &str) -> IResult<&str, &str> {
    recognize(all_consuming(ch::char('x')))(input)
}

fn recognize_clock_rate(input: &str) -> IResult<&str, &str> {
    recognize(alt((ch::char('*'), all_consuming(ch::char('x')))))(input)
}

fn recognize_n300(input: &str) -> IResult<&str, &str> {
    recognize(tag("x300"))(input)
}

fn recognize_n100(input: &str) -> IResult<&str, &str> {
    recognize(tag("x100"))(input)
}

fn recognize_n50(input: &str) -> IResult<&str, &str> {
    recognize(tag("x50"))(input)
}

fn recognize_geki(input: &str) -> IResult<&str, &str> {
    let options = (
        delimited(opt(ch::char('x')), tag("geki"), opt(ch::char('s'))),
        tag("x320"),
    );

    recognize(alt(options))(input)
}

fn recognize_katu(input: &str) -> IResult<&str, &str> {
    let options = (
        delimited(opt(ch::char('x')), tag("katu"), opt(ch::char('s'))),
        tag("x200"),
    );

    recognize(alt(options))(input)
}

fn recognize_miss(input: &str) -> IResult<&str, &str> {
    recognize(preceded(
        opt(ch::char('x')),
        preceded(ch::char('m'), opt(preceded(tag("iss"), opt(tag("es"))))),
    ))(input)
}

fn recognize_mods(input: &str) -> IResult<&str, &str> {
    delimited(ch::char('+'), parse_mods_raw, opt(ch::char('!')))(input)
}

#[derive(Debug, PartialEq)]
pub enum ParseError<'s> {
    Acc,
    Combo,
    ClockRate,
    N300,
    N100,
    N50,
    Geki,
    Katu,
    Miss,
    Mods,
    Nom(&'s str),
    Unknown(&'s str),
}

impl ParseError<'_> {
    pub fn to_str(self) -> Cow<'static, str> {
        match self {
            ParseError::Acc => "Failed to parse accuracy, must be a number".into(),
            ParseError::Combo => "Failed to parse combo, must be an integer".into(),
            ParseError::ClockRate => "Failed to parse clock rate, must be a number".into(),
            ParseError::N300 => "Failed to parse n300, must be an interger".into(),
            ParseError::N100 => "Failed to parse n100, must be an interger".into(),
            ParseError::N50 => "Failed to parse n50, must be an interger".into(),
            ParseError::Geki => "Failed to parse gekis, must be an interger".into(),
            ParseError::Katu => "Failed to parse katus, must be an interger".into(),
            ParseError::Miss => "Failed to parse misses, must be an interger".into(),
            ParseError::Mods => {
                "Failed to parse mods, must be an acronym of a mod combination".into()
            }
            ParseError::Nom(input) => format!("Failed to parse argument `{input}`").into(),
            ParseError::Unknown(input) => format!(
                "Unknown key `{input}`. Must be `mods`, `acc`, `combo`, `clockrate`, \
                `n300`, `n100`, `n50`, `miss`, `geki`, or `katu`"
            )
            .into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acc() {
        assert_eq!(
            SimulateArg::parse("acc=123.0%"),
            Ok(SimulateArg::Acc(123.0))
        );
        assert_eq!(
            SimulateArg::parse("accuracy=123"),
            Ok(SimulateArg::Acc(123.0))
        );
        assert_eq!(SimulateArg::parse("a=123%"), Ok(SimulateArg::Acc(123.0)));
        assert_eq!(SimulateArg::parse("123.0%"), Ok(SimulateArg::Acc(123.0)));
        assert_eq!(SimulateArg::parse("acc=123x"), Err(ParseError::Acc));
    }

    #[test]
    fn combo() {
        assert_eq!(
            SimulateArg::parse("combo=123x"),
            Ok(SimulateArg::Combo(123))
        );
        assert_eq!(SimulateArg::parse("c=123"), Ok(SimulateArg::Combo(123)));
        assert_eq!(SimulateArg::parse("123x"), Ok(SimulateArg::Combo(123)));
        assert_eq!(SimulateArg::parse("c=123%"), Err(ParseError::Combo));
        assert_eq!(SimulateArg::parse("combo=123x300"), Err(ParseError::Combo));
        assert_eq!(SimulateArg::parse("c=123.0x"), Err(ParseError::Combo));
    }

    #[test]
    fn clock_rate() {
        assert_eq!(
            SimulateArg::parse("clockrate=123*"),
            Ok(SimulateArg::ClockRate(123.0))
        );
        assert_eq!(
            SimulateArg::parse("cr=123.0x"),
            Ok(SimulateArg::ClockRate(123.0))
        );
        assert_eq!(
            SimulateArg::parse("cr=123.0"),
            Ok(SimulateArg::ClockRate(123.0))
        );
        assert_eq!(
            SimulateArg::parse("123.0*"),
            Ok(SimulateArg::ClockRate(123.0))
        );
        assert_eq!(
            SimulateArg::parse("123.0x"),
            Ok(SimulateArg::ClockRate(123.0))
        );
        assert_eq!(
            SimulateArg::parse("123*"),
            Ok(SimulateArg::ClockRate(123.0))
        );
        assert_eq!(SimulateArg::parse("cr=123%"), Err(ParseError::ClockRate));
    }

    #[test]
    fn n300() {
        assert_eq!(
            SimulateArg::parse("n300=123x300"),
            Ok(SimulateArg::N300(123))
        );
        assert_eq!(SimulateArg::parse("123x300"), Ok(SimulateArg::N300(123)));
        assert_eq!(SimulateArg::parse("n300=123"), Ok(SimulateArg::N300(123)));
        assert_eq!(SimulateArg::parse("n300=123x100"), Err(ParseError::N300));
    }

    #[test]
    fn n100() {
        assert_eq!(
            SimulateArg::parse("n100=123x100"),
            Ok(SimulateArg::N100(123))
        );
        assert_eq!(SimulateArg::parse("123x100"), Ok(SimulateArg::N100(123)));
        assert_eq!(SimulateArg::parse("n100=123"), Ok(SimulateArg::N100(123)));
        assert_eq!(SimulateArg::parse("n100=123x300"), Err(ParseError::N100));
    }

    #[test]
    fn n50() {
        assert_eq!(SimulateArg::parse("n50=123x50"), Ok(SimulateArg::N50(123)));
        assert_eq!(SimulateArg::parse("123x50"), Ok(SimulateArg::N50(123)));
        assert_eq!(SimulateArg::parse("n50=123"), Ok(SimulateArg::N50(123)));
        assert_eq!(SimulateArg::parse("n50=123x100"), Err(ParseError::N50));
    }

    #[test]
    fn gekis() {
        assert_eq!(
            SimulateArg::parse("ngekis=123x320"),
            Ok(SimulateArg::Geki(123))
        );
        assert_eq!(
            SimulateArg::parse("ngeki=123xgeki"),
            Ok(SimulateArg::Geki(123))
        );
        assert_eq!(
            SimulateArg::parse("gekis=123gekis"),
            Ok(SimulateArg::Geki(123))
        );
        assert_eq!(SimulateArg::parse("123x320"), Ok(SimulateArg::Geki(123)));
        assert_eq!(SimulateArg::parse("123xgekis"), Ok(SimulateArg::Geki(123)));
        assert_eq!(SimulateArg::parse("123geki"), Ok(SimulateArg::Geki(123)));
        assert_eq!(SimulateArg::parse("ngeki=123x100"), Err(ParseError::Geki));
    }

    #[test]
    fn katus() {
        assert_eq!(
            SimulateArg::parse("nkatus=123x200"),
            Ok(SimulateArg::Katu(123))
        );
        assert_eq!(
            SimulateArg::parse("nkatu=123xkatu"),
            Ok(SimulateArg::Katu(123))
        );
        assert_eq!(
            SimulateArg::parse("katus=123katus"),
            Ok(SimulateArg::Katu(123))
        );
        assert_eq!(SimulateArg::parse("123x200"), Ok(SimulateArg::Katu(123)));
        assert_eq!(SimulateArg::parse("123xkatus"), Ok(SimulateArg::Katu(123)));
        assert_eq!(SimulateArg::parse("123katu"), Ok(SimulateArg::Katu(123)));
        assert_eq!(SimulateArg::parse("nkatu=123x100"), Err(ParseError::Katu));
    }

    #[test]
    fn misses() {
        assert_eq!(
            SimulateArg::parse("misses=123xmisses"),
            Ok(SimulateArg::Miss(123))
        );
        assert_eq!(SimulateArg::parse("m=123m"), Ok(SimulateArg::Miss(123)));
        assert_eq!(SimulateArg::parse("123m"), Ok(SimulateArg::Miss(123)));
        assert_eq!(SimulateArg::parse("123xm"), Ok(SimulateArg::Miss(123)));
        assert_eq!(
            SimulateArg::parse("miss=123xmiss"),
            Ok(SimulateArg::Miss(123))
        );
        assert_eq!(SimulateArg::parse("m=123x100"), Err(ParseError::Miss));
    }

    #[test]
    fn mods() {
        assert_eq!(
            SimulateArg::parse("mods=+hdhr!"),
            Ok(SimulateArg::Mods("hdhr"))
        );
        assert_eq!(
            SimulateArg::parse("mods=+hdhr"),
            Ok(SimulateArg::Mods("hdhr"))
        );
        assert_eq!(
            SimulateArg::parse("mods=hdhr"),
            Ok(SimulateArg::Mods("hdhr"))
        );
        assert_eq!(SimulateArg::parse("+hdhr!"), Ok(SimulateArg::Mods("hdhr")));
        assert_eq!(SimulateArg::parse("+hdhr"), Ok(SimulateArg::Mods("hdhr")));

        assert_eq!(SimulateArg::parse("mods=+hdr!"), Err(ParseError::Mods));
        assert_eq!(SimulateArg::parse("mods=-hdhr!"), Err(ParseError::Mods));
        assert_eq!(SimulateArg::parse("mods=hdhr!"), Err(ParseError::Mods));
        assert_eq!(SimulateArg::parse("-hdhr!"), Err(ParseError::Nom("-hdhr!")));
        assert_eq!(SimulateArg::parse("-hdhr"), Err(ParseError::Nom("-hdhr")));
        assert_eq!(SimulateArg::parse("hdhr!"), Err(ParseError::Nom("hdhr!")));
    }
}
