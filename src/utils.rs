use nom::{
    bytes::complete::take,
    combinator::map,
    number::complete::{le_u16, le_u32, le_u64, le_u8},
    IResult,
};

// ref: https://dev.mysql.com/doc/internals/en/integer.html#packet-Protocol::LengthEncodedInteger
pub fn parse_lenenc_int<'a>(input: &'a [u8]) -> IResult<&'a [u8], u64> {
    match input[0] {
        0..0xfb => map(le_u8, |num: u8| num as u64)(input),
        0xfb | 0xfc => {
            let (i, _) = take(1usize)(input)?;
            map(le_u16, |num: u16| num as u64)(i)
        }
        0xfd => {
            let (i, _) = take(1usize)(input)?;
            let (i, v) = map(take(3usize), |s: &[u8]| {
                let mut raw = s.to_vec();
                raw.push(0);
                raw
            })(i)?;
            let (_, num) = pu32(&v).unwrap();
            Ok((i, num as u64))
        }
        0xfe => {
            let (i, _) = take(1usize)(input)?;
            le_u64(i)
        }
        0xff => unreachable!(),
    }
}

// ref: https://dev.mysql.com/doc/internals/en/string.html#packet-Protocol::LengthEncodedString
pub fn parse_lenenc_str<'a>(input: &'a [u8]) -> IResult<&'a [u8], String> {
    let (i, str_len) = parse_lenenc_int(input)?;
    map(take(str_len), |s: &[u8]| {
        String::from_utf8_lossy(s).to_string()
    })(i)
}

fn pu32(input: &[u8]) -> IResult<&[u8], u32> {
    le_u32(input)
}