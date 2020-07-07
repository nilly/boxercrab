use crate::{
    mysql::ColumnTypes,
    utils::{extract_n_string, extract_string, lenenc_int, string_fixed, take_till_term_string},
};
use nom::{
    bytes::complete::{tag, take},
    combinator::map,
    multi::{many0, many1, many_m_n},
    number::complete::{le_i64, le_u16, le_u32, le_u64, le_u8},
    sequence::tuple,
    IResult,
};

mod query;
mod rows;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct EventFlag {
    in_use: bool,
    forced_rotate: bool,
    thread_specific: bool,
    suppress_use: bool,
    update_table_map_version: bool,
    artificial: bool,
    relay_log: bool,
    ignorable: bool,
    no_filter: bool,
    mts_isolate: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Header {
    pub timestamp: u32,
    pub event_type: u8,
    pub server_id: u32,
    pub event_size: u32,
    pub log_pos: u32,
    pub flags: EventFlag,
}

pub fn parse_header(input: &[u8]) -> IResult<&[u8], Header> {
    let (i, timestamp) = le_u32(input)?;
    let (i, event_type) = le_u8(i)?;
    let (i, server_id) = le_u32(i)?;
    let (i, event_size) = le_u32(i)?;
    let (i, log_pos) = le_u32(i)?;
    let (i, flags) = map(le_u16, |f: u16| EventFlag {
        in_use: (f >> 0) % 2 == 1,
        forced_rotate: (f >> 1) % 2 == 1,
        thread_specific: (f >> 2) % 2 == 1,
        suppress_use: (f >> 3) % 2 == 1,
        update_table_map_version: (f >> 4) % 2 == 1,
        artificial: (f >> 5) % 2 == 1,
        relay_log: (f >> 6) % 2 == 1,
        ignorable: (f >> 7) % 2 == 1,
        no_filter: (f >> 8) % 2 == 1,
        mts_isolate: (f >> 9) % 2 == 1,
    })(i)?;
    Ok((
        i,
        Header {
            timestamp,
            event_type,
            server_id,
            event_size,
            log_pos,
            flags,
        },
    ))
}

pub fn check_start(i: &[u8]) -> IResult<&[u8], &[u8]> {
    tag([254, 98, 105, 110])(i)
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Event {
    // ref: https://dev.mysql.com/doc/internals/en/ignored-events.html#unknown-event
    Unknown {
        header: Header,
        checksum: u32,
    },
    // doc: https://dev.mysql.com/doc/internals/en/query-event.html
    // source: https://github.com/mysql/mysql-server/blob/a394a7e17744a70509be5d3f1fd73f8779a31424/libbinlogevents/include/statement_events.h#L44-L426
    // layout: https://github.com/mysql/mysql-server/blob/a394a7e17744a70509be5d3f1fd73f8779a31424/libbinlogevents/include/statement_events.h#L627-L643
    Query {
        header: Header,
        slave_proxy_id: u32, // thread_id
        execution_time: u32,
        schema_length: u8, // length of current select schema name
        error_code: u16,
        status_vars_length: u16,
        status_vars: Vec<query::QueryStatusVar>,
        schema: String,
        query: String,
        checksum: u32,
    },
    // ref: https://dev.mysql.com/doc/internals/en/stop-event.html
    Stop {
        header: Header,
    },
    // ref: https://dev.mysql.com/doc/internals/en/rotate-event.html
    Rotate {
        header: Header,
        position: u64,
        next_binlog: String,
    },
    // ref: https://dev.mysql.com/doc/internals/en/intvar-event.html
    IntVar {
        header: Header,
        e_type: IntVarEventType,
        value: u64,
    },
    // ref: https://dev.mysql.com/doc/internals/en/load-event.html
    Load {
        header: Header,
        thread_id: u32,
        execution_time: u32,
        skip_lines: u32,
        table_name_length: u8,
        schema_length: u8,
        num_fields: u32,
        field_term: u8,
        enclosed_by: u8,
        line_term: u8,
        line_start: u8,
        escaped_by: u8,
        opt_flags: OptFlags,
        empty_flags: EmptyFlags,
        field_name_lengths: Vec<u8>,
        field_names: Vec<String>,
        table_name: String,
        schema_name: String,
        file_name: String,
        checksum: u32,
    },
    // ref: https://dev.mysql.com/doc/internals/en/ignored-events.html#slave-event
    Slave {
        header: Header,
    },
    // ref: https://dev.mysql.com/doc/internals/en/create-file-event.html
    CreateFile {
        header: Header,
        file_id: u32,
        block_data: String,
    },
    // ref: https://dev.mysql.com/doc/internals/en/append-block-event.html
    AppendFile {
        header: Header,
        file_id: u32,
        block_data: String,
    },
    // ref: https://dev.mysql.com/doc/internals/en/exec-load-event.html
    ExecLoad {
        header: Header,
        file_id: u16,
    },
    // ref: https://dev.mysql.com/doc/internals/en/delete-file-event.html
    DeleteFile {
        header: Header,
        file_id: u16,
    },
    // ref: https://dev.mysql.com/doc/internals/en/new-load-event.html
    NewLoad {
        header: Header,
        thread_id: u32,
        execution_time: u32,
        skip_lines: u32,
        table_name_length: u8,
        schema_length: u8,
        num_fields: u32,

        field_term_length: u8,
        field_term: String,
        enclosed_by_length: u8,
        enclosed_by: String,
        line_term_length: u8,
        line_term: String,
        line_start_length: u8,
        line_start: String,
        escaped_by_length: u8,
        escaped_by: String,
        opt_flags: OptFlags,

        field_name_lengths: Vec<u8>,
        field_names: Vec<String>,
        table_name: String,
        schema_name: String,
        file_name: String,
        checksum: u32,
    },
    // ref: https://dev.mysql.com/doc/internals/en/rand-event.html
    Rand {
        header: Header,
        seed1: u64,
        seed2: u64,
    },
    // ref: https://dev.mysql.com/doc/internals/en/user-var-event.html
    // source: https://github.com/mysql/mysql-server/blob/a394a7e17744a70509be5d3f1fd73f8779a31424/libbinlogevents/include/statement_events.h#L712-L779
    // TODO ref is broken, skip
    UserVar {
        header: Header,
        // name_length: u32,
        // name: String,
        // is_null: bool,
        // d_type: Option<u8>,
        // charset: Option<u32>,
        // value_length: Option<u32>,
        // value: Option<String>,
        // flags: Option<u8>,
        unknown: Vec<u8>,
    },
    // source: https://github.com/mysql/mysql-server/blob/a394a7e17744a70509be5d3f1fd73f8779a31424/libbinlogevents/include/control_events.h#L295-L344
    // event_data layout: https://github.com/mysql/mysql-server/blob/a394a7e17744a70509be5d3f1fd73f8779a31424/libbinlogevents/include/control_events.h#L387-L416
    FormatDesc {
        header: Header,
        binlog_version: u16,
        mysql_server_version: String,
        create_timestamp: u32,
        event_header_length: u8,
        supported_types: Vec<u8>,
        checksum_alg: u8,
        checksum: u32,
    },
    XID {
        header: Header,
        xid: u64,
        checksum: u32,
    },
    // ref: https://dev.mysql.com/doc/internals/en/begin-load-query-event.html
    BeginLoadQuery {
        header: Header,
        file_id: u32,
        block_data: String,
    },
    ExecuteLoadQueryEvent {
        header: Header,
        thread_id: u32,
        execution_time: u32,
        schema_length: u8,
        error_code: u16,
        status_vars_length: u16,
        file_id: u32,
        start_pos: u32,
        end_pos: u32,
        dup_handling_flags: DupHandlingFlags,
    },
    TableMap {
        header: Header,
        // table_id take 6 bytes in buffer
        table_id: u64,
        flags: u16,
        schema_length: u8,
        schema: String,
        // [00] term sign in layout
        table_name_length: u8,
        table_name: String,
        // [00] term sign in layout
        // len encoded integer
        column_count: u64,
        columns_type: Vec<ColumnTypes>,
        // len encoded string
        column_meta_def: Vec<u8>,
        null_bits: Vec<u8>,
        checksum: u32,
    },
    // ref: https://dev.mysql.com/doc/internals/en/incident-event.html
    Incident {
        header: Header,
        d_type: IncidentEventType,
        message_length: u8,
        message: String,
    },
    // ref: https://dev.mysql.com/doc/internals/en/heartbeat-event.html
    Heartbeat {
        header: Header,
    },
    // ref: https://dev.mysql.com/doc/internals/en/rows-query-event.html
    RowQuery {
        header: Header,
        length: u8,
        query_text: String,
    },
    // source: https://github.com/mysql/mysql-server/blob/a394a7e17744a70509be5d3f1fd73f8779a31424/libbinlogevents/include/control_events.h#L932-L991
    AnonymousGtid {
        header: Header,
        rbr_only: bool,
        encoded_sig_length: u32,
        encoded_gno_length: u32,
        // FIXME unknown fields
        unknown: Vec<u8>,
        last_committed: i64,
        sequence_number: i64,
        checksum: u32,
    },
    // source: https://github.com/mysql/mysql-server/blob/a394a7e17744a70509be5d3f1fd73f8779a31424/libbinlogevents/include/control_events.h#L1073-L1103
    PreviousGtids {
        header: Header,
        // FIXME this field may be wrong
        gtid_sets: Vec<u8>,
        buf_size: u32,
        checksum: u32,
    },
    // source https://github.com/mysql/mysql-server/blob/a394a7e17744a70509be5d3f1fd73f8779a31424/libbinlogevents/include/rows_event.h#L488-L613
    WriteRowsV2 {
        header: Header,
        // table_id take 6 bytes in buffer
        table_id: u64,
        flags: rows::Flags,
        extra_data_len: u16,
        extra_data: Vec<rows::ExtraData>,
        column_count: u64,
        inserted_image_bits: Vec<u8>,
        // FIXME unknown struct field
        rows: Vec<u8>,
        checksum: u32,
    },
    UpdateRowsV2 {
        header: Header,
        // table_id take 6 bytes in buffer
        table_id: u64,
        flags: rows::Flags,
        extra_data_len: u16,
        extra_data: Vec<rows::ExtraData>,
        column_count: u64,
        before_image_bits: Vec<u8>,
        after_image_bits: Vec<u8>,
        // FIXME unknown struct field
        rows: Vec<u8>,
        checksum: u32,
    },
    DeleteRowsV2 {
        header: Header,
        // table_id take 6 bytes in buffer
        table_id: u64,
        flags: rows::Flags,
        extra_data_len: u16,
        extra_data: Vec<rows::ExtraData>,
        column_count: u64,
        deleted_image_bits: Vec<u8>,
        // FIXME unknown struct field
        rows: Vec<u8>,
        checksum: u32,
    },
}

impl Event {
    pub fn parse<'a>(input: &'a [u8]) -> IResult<&'a [u8], Event> {
        let (input, header) = parse_header(input)?;
        match header.event_type {
            0x00 => parse_unknown(input, header),
            0x02 => parse_query(input, header),
            0x03 => parse_stop(input, header),
            0x04 => parse_rotate(input, header),
            0x05 => parse_intvar(input, header),
            0x06 => parse_load(input, header),
            0x07 => parse_slave(input, header),
            0x08 => parse_create_file(input, header),
            0x09 => parse_append_file(input, header),
            0x0a => parse_exec_load(input, header),
            0x0b => parse_delete_file(input, header),
            0x0c => parse_new_load(input, header),
            0x0d => parse_rand(input, header),
            0x0e => parse_user_var(input, header),
            0x0f => parse_format_desc(input, header),
            0x10 => parse_xid(input, header),
            0x11 => parse_begin_load_query(input, header),
            0x12 => parse_execute_load_query(input, header),
            0x13 => parse_table_map(input, header),
            0x1a => parse_incident(input, header),
            0x1b => parse_heartbeat(input, header),
            0x1d => parse_row_query(input, header),
            0x14..=0x19 => unreachable!(),
            0x1e => parse_write_rows_v2(input, header),
            0x1f => parse_update_rows_v2(input, header),
            0x20 => parse_delete_rows_v2(input, header),
            0x22 => parse_anonymous_gtid(input, header),
            0x23 => parse_previous_gtids(input, header),
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum IntVarEventType {
    InvalidIntEvent,
    LastInsertIdEvent,
    InsertIdEvent,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct EmptyFlags {
    field_term_empty: bool,
    enclosed_empty: bool,
    line_term_empty: bool,
    line_start_empty: bool,
    escape_empty: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct OptFlags {
    dump_file: bool,
    opt_enclosed: bool,
    replace: bool,
    ignore: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum DupHandlingFlags {
    Error,
    Ignore,
    Replace,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum IncidentEventType {
    None,
    LostEvents,
}

fn pu64(input: &[u8]) -> IResult<&[u8], u64> {
    le_u64(input)
}

// TODO this function hasn't been tested yet
pub fn parse_unknown<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    map(le_u32, move |checksum: u32| Event::Unknown {
        header: header.clone(),
        checksum,
    })(input)
}

fn parse_query<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, slave_proxy_id) = le_u32(input)?;
    let (i, execution_time) = le_u32(i)?;
    let (i, schema_length) = le_u8(i)?;
    let (i, error_code) = le_u16(i)?;
    let (i, status_vars_length) = le_u16(i)?;
    let (i, raw_vars) = take(status_vars_length)(i)?;
    let (remain, status_vars) = many0(query::parse_status_var)(raw_vars)?;
    assert_eq!(remain.len(), 0);
    let (i, schema) = map(take(schema_length), |s: &[u8]| {
        String::from_utf8(s[0..schema_length as usize].to_vec()).unwrap()
    })(i)?;
    let (i, _) = take(1usize)(i)?;
    let (i, query) = map(
        take(
            header.event_size
                - 19
                - 4
                - 4
                - 1
                - 2
                - 2
                - status_vars_length as u32
                - schema_length as u32
                - 1
                - 4,
        ),
        |s: &[u8]| extract_string(s),
    )(i)?;
    let (i, checksum) = le_u32(i)?;
    Ok((
        i,
        Event::Query {
            header,
            slave_proxy_id,
            execution_time,
            schema_length,
            error_code,
            status_vars_length,
            status_vars,
            schema,
            query,
            checksum,
        },
    ))
}

pub fn parse_stop<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    Ok((input, Event::Stop { header }))
}

pub fn parse_rotate<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, position) = le_u64(input)?;
    let str_len = header.event_size - 19 - 8;
    let (i, next_binlog) = map(take(str_len), |s: &[u8]| {
        extract_n_string(i, str_len as usize)
    })(i)?;
    Ok((
        i,
        Event::Rotate {
            header,
            position,
            next_binlog,
        },
    ))
}

pub fn parse_intvar<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, e_type) = map(le_u8, |t: u8| match t {
        0x00 => IntVarEventType::InvalidIntEvent,
        0x01 => IntVarEventType::LastInsertIdEvent,
        0x02 => IntVarEventType::InsertIdEvent,
        _ => unreachable!(),
    })(input)?;
    let (i, value) = le_u64(i)?;
    Ok((
        i,
        Event::IntVar {
            header,
            e_type,
            value,
        },
    ))
}

fn extract_many_fields<'a>(
    input: &'a [u8],
    header: &Header,
    num_fields: u32,
    table_name_length: u8,
    schema_length: u8,
) -> IResult<&'a [u8], (Vec<u8>, Vec<String>, String, String, String)> {
    let (i, field_name_lengths) = map(take(num_fields), |s: &[u8]| s.to_vec())(input)?;
    let total_len: u64 = field_name_lengths.iter().sum::<u8>() as u64 + num_fields as u64;
    let (i, raw_field_names) = take(total_len)(i)?;
    let (i, field_names) = many_m_n(
        num_fields as usize,
        num_fields as usize,
        take_till_term_string,
    )(raw_field_names)?;
    let (i, table_name) = map(take(table_name_length + 1), |s: &[u8]| extract_string(s))(i)?;
    let (i, schema_name) = map(take(schema_length + 1), |s: &[u8]| extract_string(s))(i)?;
    let (i, file_name) = map(
        take(
            header.event_size as usize
                - 19
                - 25
                - num_fields as usize
                - total_len as usize
                - table_name_length as usize
                - schema_length as usize
                - 3
                - 4,
        ),
        |s: &[u8]| extract_string(s),
    )(i)?;
    Ok((
        i,
        (
            field_name_lengths,
            field_names,
            table_name,
            schema_name,
            file_name,
        ),
    ))
}

pub fn parse_load<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (
        i,
        (
            thread_id,
            execution_time,
            skip_lines,
            table_name_length,
            schema_length,
            num_fields,
            field_term,
            enclosed_by,
            line_term,
            line_start,
            escaped_by,
        ),
    ) = tuple((
        le_u32, le_u32, le_u32, le_u8, le_u8, le_u32, le_u8, le_u8, le_u8, le_u8, le_u8,
    ))(input)?;
    let (i, opt_flags) = map(le_u8, |flags: u8| OptFlags {
        dump_file: (flags >> 0) % 2 == 1,
        opt_enclosed: (flags >> 1) % 1 == 1,
        replace: (flags >> 2) % 2 == 1,
        ignore: (flags >> 3) % 2 == 1,
    })(i)?;
    let (i, empty_flags) = map(le_u8, |flags: u8| EmptyFlags {
        field_term_empty: (flags >> 0) % 2 == 1,
        enclosed_empty: (flags >> 1) % 2 == 1,
        line_term_empty: (flags >> 2) % 2 == 1,
        line_start_empty: (flags >> 3) % 2 == 1,
        escape_empty: (flags >> 4) % 2 == 1,
    })(i)?;
    let (i, (field_name_lengths, field_names, table_name, schema_name, file_name)) =
        extract_many_fields(i, &header, num_fields, table_name_length, schema_length)?;
    let (i, checksum) = le_u32(i)?;
    Ok((
        i,
        Event::Load {
            header,
            thread_id,
            execution_time,
            skip_lines,
            table_name_length,
            schema_length,
            num_fields,
            field_term,
            enclosed_by,
            line_term,
            line_start,
            escaped_by,
            opt_flags,
            empty_flags,
            field_name_lengths,
            field_names,
            table_name,
            schema_name,
            file_name,
            checksum,
        },
    ))
}

pub fn parse_slave<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    Ok((input, Event::Slave { header }))
}

fn parse_file_data<'a>(input: &'a [u8], header: &Header) -> IResult<&'a [u8], (u32, String)> {
    let (i, file_id) = le_u32(input)?;
    let (i, block_data) = map(take(header.event_size - 19 - 4), |s: &[u8]| {
        extract_string(s)
    })(i)?;
    Ok((i, (file_id, block_data)))
}

pub fn parse_create_file<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, (file_id, block_data)) = parse_file_data(input, &header)?;
    Ok((
        i,
        Event::CreateFile {
            header,
            file_id,
            block_data,
        },
    ))
}

pub fn parse_append_file<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, (file_id, block_data)) = parse_file_data(input, &header)?;
    Ok((
        i,
        Event::AppendFile {
            header,
            file_id,
            block_data,
        },
    ))
}

pub fn parse_exec_load<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    map(le_u16, |file_id: u16| Event::ExecLoad {
        header: header.clone(),
        file_id,
    })(input)
}

pub fn parse_delete_file<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    map(le_u16, |file_id: u16| Event::DeleteFile {
        header: header.clone(),
        file_id,
    })(input)
}

fn extract_from_prev<'a>(input: &'a [u8]) -> IResult<&'a [u8], (u8, String)> {
    let (i, len) = le_u8(input)?;
    map(take(len), move |s| (len, extract_n_string(s, len as usize)))(i)
}

pub fn parse_new_load<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, (thread_id, execution_time, skip_lines, table_name_length, schema_length, num_fields)) =
        tuple((le_u32, le_u32, le_u32, le_u8, le_u8, le_u32))(input)?;
    let (i, (field_term_length, field_term)) = extract_from_prev(i)?;
    let (i, (enclosed_by_length, enclosed_by)) = extract_from_prev(i)?;
    let (i, (line_term_length, line_term)) = extract_from_prev(i)?;
    let (i, (line_start_length, line_start)) = extract_from_prev(i)?;
    let (i, (escaped_by_length, escaped_by)) = extract_from_prev(i)?;
    let (i, opt_flags) = map(le_u8, |flags| OptFlags {
        dump_file: (flags >> 0) % 2 == 1,
        opt_enclosed: (flags >> 1) % 2 == 1,
        replace: (flags >> 2) % 2 == 1,
        ignore: (flags >> 3) % 2 == 1,
    })(i)?;
    let (i, (field_name_lengths, field_names, table_name, schema_name, file_name)) =
        extract_many_fields(i, &header, num_fields, table_name_length, schema_length)?;
    let (i, checksum) = le_u32(i)?;
    Ok((
        i,
        Event::NewLoad {
            header,
            thread_id,
            execution_time,
            skip_lines,
            table_name_length,
            schema_length,
            num_fields,
            field_name_lengths,
            field_term,
            enclosed_by_length,
            enclosed_by,
            line_term_length,
            line_term,
            line_start_length,
            line_start,
            escaped_by_length,
            escaped_by,
            opt_flags,
            field_term_length,
            field_names,
            table_name,
            schema_name,
            file_name,
            checksum,
        },
    ))
}

pub fn parse_rand<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, (seed1, seed2)) = tuple((le_u64, le_u64))(input)?;
    Ok((
        i,
        Event::Rand {
            header,
            seed1,
            seed2,
        },
    ))
}

pub fn parse_user_var<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, unknown) = map(take(header.event_size - 19), |s: &[u8]| s.to_vec())(input)?;
    Ok((i, Event::UserVar { header, unknown }))
}

fn parse_format_desc<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, binlog_version) = le_u16(input)?;
    let (i, mysql_server_version) = map(take(50usize), |s: &[u8]| extract_string(s))(i)?;
    let (i, create_timestamp) = le_u32(i)?;
    let (i, event_header_length) = le_u8(i)?;
    let num = header.event_size - 19 - (2 + 50 + 4 + 1) - 1 - 4;
    let (i, supported_types) = map(take(num), |s: &[u8]| s.to_vec())(i)?;
    let (i, checksum_alg) = le_u8(i)?;
    let (i, checksum) = le_u32(i)?;
    Ok((
        i,
        Event::FormatDesc {
            header,
            binlog_version,
            mysql_server_version,
            create_timestamp,
            event_header_length,
            supported_types,
            checksum_alg,
            checksum,
        },
    ))
}

pub fn parse_xid<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, (xid, checksum)) = tuple((le_u64, le_u32))(input)?;
    Ok((
        i,
        Event::XID {
            header,
            xid,
            checksum,
        },
    ))
}

pub fn parse_begin_load_query<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, (file_id, block_data)) = parse_file_data(input, &header)?;
    Ok((
        i,
        Event::BeginLoadQuery {
            header,
            file_id,
            block_data,
        },
    ))
}

pub fn parse_execute_load_query<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (
        i,
        (
            thread_id,
            execution_time,
            schema_length,
            error_code,
            status_vars_length,
            file_id,
            start_pos,
            end_pos,
        ),
    ) = tuple((
        le_u32, le_u32, le_u8, le_u16, le_u16, le_u32, le_u32, le_u32,
    ))(input)?;
    let (i, dup_handling_flags) = map(le_u8, |flags| match flags {
        0 => DupHandlingFlags::Error,
        1 => DupHandlingFlags::Ignore,
        2 => DupHandlingFlags::Replace,
        _ => unreachable!(),
    })(i)?;
    Ok((
        i,
        Event::ExecuteLoadQueryEvent {
            header,
            thread_id,
            execution_time,
            schema_length,
            error_code,
            status_vars_length,
            file_id,
            start_pos,
            end_pos,
            dup_handling_flags,
        },
    ))
}

fn parse_table_map<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, table_id): (&'a [u8], u64) = map(take(6usize), |id_raw: &[u8]| {
        let mut filled = id_raw.to_vec();
        filled.extend(vec![0, 0]);
        pu64(&filled).unwrap().1
    })(input)?;
    // Reserved for future use; currently always 0
    let (i, flags) = le_u16(i)?;
    let (i, (schema_length, schema)) = string_fixed(i)?;
    let (i, term) = le_u8(i)?;
    assert_eq!(term, 0);

    let (i, (table_name_length, table_name)) = string_fixed(i)?;
    let (i, term) = le_u8(i)?;
    assert_eq!(term, 0);
    let (i, (_, column_count)) = lenenc_int(i)?;
    let (i, columns_type) = map(take(column_count), |s: &[u8]| {
        s.iter().map(|&t| ColumnTypes::from_u8(t)).collect()
    })(i)?;
    let (i, (_, column_meta_count)) = lenenc_int(i)?;
    let (i, column_meta_def) = map(take(column_meta_count), |s: &[u8]| s.to_vec())(i)?;
    let mask_len = (column_count + 7) / 8;
    dbg!(&mask_len);
    let (i, null_bits) = map(take(mask_len), |s: &[u8]| s.to_vec())(i)?;
    let (i, checksum) = le_u32(i)?;
    Ok((
        i,
        Event::TableMap {
            header,
            table_id,
            flags,
            schema_length,
            schema,
            table_name_length,
            table_name,
            column_count,
            columns_type,
            column_meta_def,
            null_bits,
            checksum,
        },
    ))
}

pub fn parse_incident<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, d_type) = map(le_u16, |t| match t {
        0x0000 => IncidentEventType::None,
        0x0001 => IncidentEventType::LostEvents,
        _ => unreachable!(),
    })(input)?;
    let (i, message_length) = le_u8(i)?;
    let (i, message) = map(take(message_length), |s: &[u8]| {
        extract_n_string(s, message_length as usize)
    })(i)?;
    Ok((
        i,
        Event::Incident {
            header,
            d_type,
            message_length,
            message,
        },
    ))
}

pub fn parse_heartbeat<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    Ok((input, Event::Heartbeat { header }))
}

pub fn parse_row_query<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, length) = le_u8(input)?;
    let (i, query_text) = map(take(length), |s: &[u8]| {
        extract_n_string(s, length as usize)
    })(i)?;
    Ok((
        i,
        Event::RowQuery {
            header,
            length,
            query_text,
        },
    ))
}

fn parse_anonymous_gtid<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, rbr_only) = map(le_u8, |t: u8| t == 0)(input)?;
    let (i, encoded_sig_length) = le_u32(i)?;
    let (i, encoded_gno_length) = le_u32(i)?;
    let (i, unknown) = map(
        take(header.event_size - 19 - (1 + 4 * 2 + 8 * 2 + 4)),
        |s: &[u8]| s.to_vec(),
    )(i)?;
    let (i, last_committed) = le_i64(i)?;
    let (i, sequence_number) = le_i64(i)?;
    let (i, checksum) = le_u32(i)?;
    Ok((
        i,
        Event::AnonymousGtid {
            header,
            rbr_only,
            encoded_sig_length,
            encoded_gno_length,
            last_committed,
            sequence_number,
            unknown,
            checksum,
        },
    ))
}

fn parse_previous_gtids<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, gtid_sets) = map(take(header.event_size - 19 - 4 - 4), |s: &[u8]| s.to_vec())(input)?;
    let (i, buf_size) = le_u32(i)?;
    let (i, checksum) = le_u32(i)?;
    Ok((
        i,
        Event::PreviousGtids {
            header,
            gtid_sets,
            buf_size,
            checksum,
        },
    ))
}

fn parse_half_row<'a>(
    input: &'a [u8],
) -> IResult<&'a [u8], (u64, rows::Flags, u16, Vec<rows::ExtraData>, (usize, u64))> {
    let (i, table_id): (&'a [u8], u64) = map(take(6usize), |id_raw: &[u8]| {
        let mut filled = id_raw.to_vec();
        filled.extend(vec![0, 0]);
        pu64(&filled).unwrap().1
    })(input)?;
    let (i, flags) = map(le_u16, |flag: u16| rows::Flags {
        end_of_stmt: (flag >> 0) % 2 == 1,
        foreign_key_checks: (flag >> 1) % 2 == 0,
        unique_key_checks: (flag >> 2) % 2 == 0,
        has_columns: (flag >> 3) % 2 == 0,
    })(i)?;
    let (i, extra_data_len) = le_u16(i)?;
    assert!(extra_data_len >= 2);
    let (i, extra_data) = match extra_data_len {
        2 => (i, vec![]),
        _ => many1(rows::parse_extra_data)(i)?,
    };

    // parse body
    let (i, (encode_len, column_count)) = lenenc_int(i)?;
    Ok((
        i,
        (
            table_id,
            flags,
            extra_data_len,
            extra_data,
            (encode_len, column_count),
        ),
    ))
}

pub fn parse_write_rows_v2<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, (table_id, flags, extra_data_len, extra_data, (encode_len, column_count))) =
        parse_half_row(input)?;

    let (i, inserted_image_bits) = map(take((column_count + 7) / 8), |s: &[u8]| s.to_vec())(i)?;
    let (i, rows) = map(
        take(
            header.event_size
                - 19
                - 6
                - 2
                - 2
                - (extra_data_len as u32 - 2)
                - encode_len as u32
                - ((column_count as u32 + 7) / 8)
                - 4,
        ),
        |s: &[u8]| s.to_vec(),
    )(i)?;
    let (i, checksum) = le_u32(i)?;
    Ok((
        i,
        Event::WriteRowsV2 {
            header,
            table_id,
            flags,
            extra_data_len,
            extra_data,
            column_count,
            inserted_image_bits,
            rows,
            checksum,
        },
    ))
}

pub fn parse_delete_rows_v2<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, (table_id, flags, extra_data_len, extra_data, (encode_len, column_count))) =
        parse_half_row(input)?;

    let (i, deleted_image_bits) = map(take((column_count + 7) / 8), |s: &[u8]| s.to_vec())(i)?;
    let (i, rows) = map(
        take(
            header.event_size
                - 19
                - 6
                - 2
                - 2
                - (extra_data_len as u32 - 2)
                - encode_len as u32
                - ((column_count as u32 + 7) / 8)
                - 4,
        ),
        |s: &[u8]| s.to_vec(),
    )(i)?;
    let (i, checksum) = le_u32(i)?;
    Ok((
        i,
        Event::DeleteRowsV2 {
            header,
            table_id,
            flags,
            extra_data_len,
            extra_data,
            column_count,
            deleted_image_bits,
            rows,
            checksum,
        },
    ))
}

pub fn parse_update_rows_v2<'a>(input: &'a [u8], header: Header) -> IResult<&'a [u8], Event> {
    let (i, (table_id, flags, extra_data_len, extra_data, (encode_len, column_count))) =
        parse_half_row(input)?;

    let (i, before_image_bits) = map(take((column_count + 7) / 8), |s: &[u8]| s.to_vec())(i)?;
    let (i, after_image_bits) = map(take((column_count + 7) / 8), |s: &[u8]| s.to_vec())(i)?;
    let (i, rows) = map(
        take(
            header.event_size
                - 19
                - 6
                - 2
                - 2
                - (extra_data_len as u32 - 2)
                - encode_len as u32
                - ((column_count as u32 + 7) / 8) * 2
                - 4,
        ),
        |s: &[u8]| s.to_vec(),
    )(i)?;
    let (i, checksum) = le_u32(i)?;
    Ok((
        i,
        Event::UpdateRowsV2 {
            header,
            table_id,
            flags,
            extra_data_len,
            extra_data,
            column_count,
            before_image_bits,
            after_image_bits,
            rows,
            checksum,
        },
    ))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_anonymous_gtids() {
        use super::parse_header;
        let input: Vec<u8> = vec![
            54, 157, 253, 94, 34, 123, 0, 0, 0, 65, 0, 0, 0, 219, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 1,
            0, 0, 0, 0, 0, 0, 0, 10, 21, 198, 18,
        ];
        let (i, header) = parse_header(&input).unwrap();
        let (i, event) = parse_anonymous_gtid(i, header).unwrap();
        match event {
            Event::AnonymousGtid {
                last_committed,
                sequence_number,
                rbr_only,
                ..
            } => {
                assert_eq!(last_committed, 0);
                assert_eq!(sequence_number, 1);
                assert_eq!(rbr_only, false);
                assert_eq!(i.len(), 0);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_format_desc() {
        use super::parse_header;
        let input: Vec<u8> = vec![
            220, 156, 253, 94, 15, 123, 0, 0, 0, 119, 0, 0, 0, 123, 0, 0, 0, 1, 0, 4, 0, 53, 46,
            55, 46, 50, 57, 45, 108, 111, 103, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 220, 156, 253, 94,
            19, 56, 13, 0, 8, 0, 18, 0, 4, 4, 4, 4, 18, 0, 0, 95, 0, 4, 26, 8, 0, 0, 0, 8, 8, 8, 2,
            0, 0, 0, 10, 10, 10, 42, 42, 0, 18, 52, 0, 1, 207, 88, 126, 238,
        ];
        let (i, header) = parse_header(&input).unwrap();
        let (i, event) = parse_format_desc(i, header).unwrap();
        match event {
            Event::FormatDesc {
                binlog_version,
                mysql_server_version,
                create_timestamp,
                ..
            } => {
                assert_eq!(binlog_version, 4);
                assert_eq!(mysql_server_version, "5.7.29-log");
                assert_eq!(create_timestamp, 1593679068);
                assert_eq!(i.len(), 0);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_xid() {
        let input: Vec<u8> = vec![
            170, 157, 253, 94, 16, 123, 0, 0, 0, 31, 0, 0, 0, 71, 3, 0, 0, 0, 0, 11, 0, 0, 0, 0, 0,
            0, 0, 188, 120, 235, 134,
        ];
        let (i, header) = parse_header(&input).unwrap();
        let (i, e) = parse_xid(i, header).unwrap();
        match e {
            Event::XID { xid, checksum, .. } => {
                assert_eq!(i.len(), 0);
                assert_eq!(xid, 11);
                assert_eq!(checksum, 0x86eb78bc);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_previous_gtids() {
        use super::parse_header;

        let input: Vec<u8> = vec![
            220, 156, 253, 94, 35, 123, 0, 0, 0, 31, 0, 0, 0, 154, 0, 0, 0, 128, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 82, 75, 196, 253,
        ];
        let (i, header) = parse_header(&input).unwrap();
        let (i, _) = parse_previous_gtids(i, header).unwrap();
        assert_eq!(i.len(), 0);
        // TODO do more parse
    }

    #[test]
    fn test_table_map() {
        use super::parse_header;

        let input: Vec<u8> = vec![
            170, 157, 253, 94, 19, 123, 0, 0, 0, 60, 0, 0, 0, 246, 2, 0, 0, 0, 0, 109, 0, 0, 0, 0,
            0, 1, 0, 4, 116, 101, 115, 116, 0, 10, 114, 117, 110, 111, 111, 98, 95, 116, 98, 108,
            0, 4, 3, 15, 15, 10, 4, 44, 1, 120, 0, 8, 194, 168, 53, 68,
        ];
        let (i, header) = parse_header(&input).unwrap();
        let (i, event) = parse_table_map(i, header).unwrap();
        match event {
            Event::TableMap {
                table_id,
                schema,
                checksum,
                ..
            } => {
                assert_eq!(i.len(), 0);
                // TODO do more checks here
                assert_eq!(table_id, 109);
                assert_eq!(schema, "test".to_string());
                assert_eq!(checksum, 0x4435a8c2);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_query() {
        use super::parse_header;

        let input: Vec<u8> = vec![
            54, 157, 253, 94, 2, 123, 0, 0, 0, 78, 1, 0, 0, 41, 2, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0,
            0, 4, 0, 0, 33, 0, 0, 0, 0, 0, 0, 1, 32, 0, 160, 85, 0, 0, 0, 0, 6, 3, 115, 116, 100,
            4, 33, 0, 33, 0, 224, 0, 12, 1, 116, 101, 115, 116, 0, 116, 101, 115, 116, 0, 67, 82,
            69, 65, 84, 69, 32, 84, 65, 66, 76, 69, 32, 73, 70, 32, 78, 79, 84, 32, 69, 88, 73, 83,
            84, 83, 32, 96, 114, 117, 110, 111, 111, 98, 95, 116, 98, 108, 96, 40, 10, 32, 32, 32,
            96, 114, 117, 110, 111, 111, 98, 95, 105, 100, 96, 32, 73, 78, 84, 32, 85, 78, 83, 73,
            71, 78, 69, 68, 32, 65, 85, 84, 79, 95, 73, 78, 67, 82, 69, 77, 69, 78, 84, 44, 10, 32,
            32, 32, 96, 114, 117, 110, 111, 111, 98, 95, 116, 105, 116, 108, 101, 96, 32, 86, 65,
            82, 67, 72, 65, 82, 40, 49, 48, 48, 41, 32, 78, 79, 84, 32, 78, 85, 76, 76, 44, 10, 32,
            32, 32, 96, 114, 117, 110, 111, 111, 98, 95, 97, 117, 116, 104, 111, 114, 96, 32, 86,
            65, 82, 67, 72, 65, 82, 40, 52, 48, 41, 32, 78, 79, 84, 32, 78, 85, 76, 76, 44, 10, 32,
            32, 32, 96, 115, 117, 98, 109, 105, 115, 115, 105, 111, 110, 95, 100, 97, 116, 101, 96,
            32, 68, 65, 84, 69, 44, 10, 32, 32, 32, 80, 82, 73, 77, 65, 82, 89, 32, 75, 69, 89, 32,
            40, 32, 96, 114, 117, 110, 111, 111, 98, 95, 105, 100, 96, 32, 41, 10, 41, 69, 78, 71,
            73, 78, 69, 61, 73, 110, 110, 111, 68, 66, 32, 68, 69, 70, 65, 85, 76, 84, 32, 67, 72,
            65, 82, 83, 69, 84, 61, 117, 116, 102, 56, 120, 116, 234, 84,
        ];
        let (i, header) = parse_header(&input).unwrap();
        let (i, event) = parse_query(i, header.clone()).unwrap();
        assert_eq!(i.len(), 0);
        assert_eq!(
        event,
        Event::Query {
            header,
            slave_proxy_id: 3,
            execution_time: 0,
            schema_length: 4,
            schema: String::from("test"),
            error_code: 0,
            status_vars_length: 33,
            status_vars: vec![
                query::QueryStatusVar::Q_FLAGS2_CODE(query::Q_FLAGS2_CODE_VAL {
                    auto_is_null: false,
                    auto_commit: true,
                    foreign_key_checks: true,
                    unique_checks: true,
                }),
                query::QueryStatusVar::Q_SQL_MODE_CODE(query::Q_SQL_MODE_CODE_VAL {
                    real_as_float: false,
                    pipes_as_concat: false,
                    ansi_quotes: false,
                    ignore_space: false,
                    not_used: false,
                    only_full_group_by: true,
                    no_unsigned_subtraction: false,
                    no_dir_in_create: false,
                    postgresql: false,
                    oracle: false,
                    mssql: false,
                    db2: false,
                    maxdb: false,
                    no_key_options: false,
                    no_table_options: false,
                    no_field_options: false,
                    mysql323: false,
                    mysql40: false,
                    ansi: false,
                    no_auto_value_on_zero: false,
                    no_backslash_escapes: false,
                    strict_trans_tables: true,
                    strict_all_tables: false,
                    no_zero_in_date: true,
                    no_zero_date: true,
                    invalid_dates: false,
                    error_for_division_by_zero: true,
                    traditional: false,
                    no_auto_create_user: true,
                    high_not_precedence: false,
                    no_engine_substitution: true,
                    pad_char_to_full_length: false
                }),
                query::QueryStatusVar::Q_CATALOG_NZ_CODE("std".to_string()),
                query::QueryStatusVar::Q_CHARSET_CODE(33, 33, 224),
                query::QueryStatusVar::Q_UPDATED_DB_NAMES(vec!["test".to_string()])
            ],
            query: String::from("CREATE TABLE IF NOT EXISTS `runoob_tbl`(\n   `runoob_id` INT UNSIGNED AUTO_INCREMENT,\n   `runoob_title` VARCHAR(100) NOT NULL,\n   `runoob_author` VARCHAR(40) NOT NULL,\n   `submission_date` DATE,\n   PRIMARY KEY ( `runoob_id` )\n)ENGINE=InnoDB DEFAULT CHARSET=utf8"),
            checksum: 1424651384,
        }
    );
    }

    #[test]
    fn test_write_row_v2() {
        let input: Vec<u8> = vec![
            170, 157, 253, 94, 30, 123, 0, 0, 0, 50, 0, 0, 0, 40, 3, 0, 0, 0, 0, 109, 0, 0, 0, 0,
            0, 1, 0, 2, 0, 4, 255, 240, 1, 0, 0, 0, 2, 0, 120, 100, 2, 103, 115, 226, 200, 15, 201,
            254, 227, 34,
        ];
        let (i, header) = parse_header(&input).unwrap();
        let (i, e) = parse_write_rows_v2(&i, header).unwrap();
        match e {
            Event::WriteRowsV2 {
                table_id,
                flags,
                checksum,
                ..
            } => {
                assert_eq!(dbg!(i).len(), 0);
                assert_eq!(table_id, 109);
                assert_eq!(checksum, 0x22e3fec9);
                assert_eq!(
                    flags,
                    rows::Flags {
                        end_of_stmt: true,
                        foreign_key_checks: true,
                        unique_key_checks: true,
                        has_columns: true
                    }
                )
            }
            _ => unreachable!(),
        }
    }
}
