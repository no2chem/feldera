use crate::{
    format::{Encoder, InputFormat, OutputFormat, ParseError, Parser},
    util::{split_on_newline, truncate_ellipse},
    ControllerError, DeCollectionHandle, OutputConsumer, SerBatch,
};
use actix_web::HttpRequest;
use anyhow::{bail, Result as AnyResult};
use csv::{
    Reader as CsvReader, ReaderBuilder as CsvReaderBuilder, WriterBuilder as CsvWriterBuilder,
};
use erased_serde::{Deserializer as ErasedDeserializer, Serialize as ErasedSerialize};
use serde::{Deserialize, Serialize};
use serde_urlencoded::Deserializer as UrlDeserializer;
use serde_yaml::Value as YamlValue;
use std::{borrow::Cow, io::Read, mem::take, sync::Arc};
use utoipa::ToSchema;

mod deserializer;
pub use deserializer::byte_record_deserializer;
pub use deserializer::string_record_deserializer;

/// When including a long CSV record in an error message,
/// truncate it to `MAX_RECORD_LEN_IN_ERRMSG` bytes.
static MAX_RECORD_LEN_IN_ERRMSG: usize = 4096;

/// CSV format parser.
pub struct CsvInputFormat;

#[derive(Deserialize, Serialize, ToSchema)]
pub struct CsvParserConfig {}

impl InputFormat for CsvInputFormat {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("csv")
    }

    /// Create a parser using configuration extracted from an HTTP request.
    // We could just rely on serde to deserialize the config from the
    // HTTP query, but a specialized method gives us more flexibility.
    fn config_from_http_request(
        &self,
        _endpoint_name: &str,
        _request: &HttpRequest,
    ) -> Result<Box<dyn ErasedSerialize>, ControllerError> {
        Ok(Box::new(CsvParserConfig {}))
    }

    fn new_parser(
        &self,
        _endpoint_name: &str,
        input_stream: &dyn DeCollectionHandle,
        _config: &YamlValue,
    ) -> Result<Box<dyn Parser>, ControllerError> {
        Ok(Box::new(CsvParser::new(input_stream)) as Box<dyn Parser>)
    }
}

struct CsvParser {
    /// Input handle to push parsed data to.
    input_stream: Box<dyn DeCollectionHandle>,

    /// Since we cannot assume that the input buffer ends on line end,
    /// we save the "leftover" part of the buffer after the last new-line
    /// character and prepend it to the next input buffer.
    leftover: Vec<u8>,

    /// Builder used to create a new CSV reader for each received data
    /// buffer.
    builder: CsvReaderBuilder,

    last_event_number: u64,
}

impl CsvParser {
    fn new(input_stream: &dyn DeCollectionHandle) -> Self {
        let mut builder = CsvReaderBuilder::new();
        builder.has_headers(false);

        Self {
            input_stream: input_stream.fork(),
            leftover: Vec::new(),
            builder,
            last_event_number: 0,
        }
    }

    fn parse_from_reader<R>(&mut self, mut reader: CsvReader<R>) -> (usize, Vec<ParseError>)
    where
        R: Read,
    {
        let mut errors = Vec::new();
        let mut num_records = 0;

        for record in reader.byte_records() {
            match record {
                Err(e) => {
                    // TODO: extract invalid CSV record from the reader, so we can report it with
                    // `ParseError::text_event_error`.
                    errors.push(ParseError::text_event_error(
                        "failed to deserialize CSV record",
                        e,
                        self.last_event_number + 1,
                        None,
                        None,
                    ));
                }
                Ok(record) => {
                    let mut deserializer = byte_record_deserializer(&record, None);
                    let mut deserializer = <dyn ErasedDeserializer>::erase(&mut deserializer);
                    match self.input_stream.insert(&mut deserializer) {
                        Err(e) => {
                            errors.push(ParseError::text_event_error(
                                "failed to deserialize CSV record",
                                e,
                                self.last_event_number + 1,
                                Some(&format!("{record:?}")),
                                None,
                            ));
                        }
                        Ok(()) => {
                            num_records += 1;
                        }
                    }
                }
            }

            self.last_event_number += 1;
        }

        self.input_stream.flush();
        (num_records, errors)
    }
}

impl Parser for CsvParser {
    fn input_fragment(&mut self, data: &[u8]) -> (usize, Vec<ParseError>) {
        // println!("input {} bytes:\n{}\nself.leftover:\n{}", data.len(),
        //    std::str::from_utf8(data).map(|s| s.to_string()).unwrap_or_else(|e|
        // format!("invalid csv: {e}")),    std::str::from_utf8(&self.leftover).
        // map(|s| s.to_string()).unwrap_or_else(|e| format!("invalid csv: {e}")));

        let leftover = split_on_newline(data);

        // println!("leftover: {leftover}");

        if leftover == 0 {
            // `data` doesn't contain a new-line character; append it to
            // the `leftover` buffer so it gets processed with the next input
            // buffer.
            self.leftover.extend_from_slice(data);
            (0, Vec::new())
        } else {
            let mut leftover_buf = take(&mut self.leftover);
            let reader = self
                .builder
                .from_reader(Read::chain(leftover_buf.as_slice(), &data[0..leftover]));

            let res = self.parse_from_reader(reader);
            // println!("parse returned: {res:?}");

            leftover_buf.clear();
            leftover_buf.extend_from_slice(&data[leftover..]);
            self.leftover = leftover_buf;

            res
        }
    }

    fn eoi(&mut self) -> (usize, Vec<ParseError>) {
        if self.leftover.is_empty() {
            return (0, Vec::new());
        }

        // Try to interpret the leftover chunk as a complete CSV line.
        let mut leftover_buf = take(&mut self.leftover);
        let reader = self.builder.from_reader(leftover_buf.as_slice());

        let res = self.parse_from_reader(reader);
        leftover_buf.clear();
        self.leftover = leftover_buf;
        res
    }

    fn fork(&self) -> Box<dyn Parser> {
        Box::new(Self::new(&*self.input_stream))
    }
}

/// CSV format encoder.
pub struct CsvOutputFormat;

const fn default_buffer_size_records() -> usize {
    10_000
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct CsvEncoderConfig {
    #[serde(default = "default_buffer_size_records")]
    buffer_size_records: usize,
}

impl OutputFormat for CsvOutputFormat {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("csv")
    }

    fn config_from_http_request(
        &self,
        endpoint_name: &str,
        request: &HttpRequest,
    ) -> Result<Box<dyn ErasedSerialize>, ControllerError> {
        Ok(Box::new(
            CsvEncoderConfig::deserialize(UrlDeserializer::new(form_urlencoded::parse(
                request.query_string().as_bytes(),
            )))
            .map_err(|e| {
                ControllerError::encoder_config_parse_error(
                    endpoint_name,
                    &e,
                    request.query_string(),
                )
            })?,
        ))
    }

    fn new_encoder(
        &self,
        config: &YamlValue,
        consumer: Box<dyn OutputConsumer>,
    ) -> AnyResult<Box<dyn Encoder>> {
        let config = CsvEncoderConfig::deserialize(config)?;

        Ok(Box::new(CsvEncoder::new(consumer, config)))
    }
}

struct CsvEncoder {
    /// Input handle to push serialized data to.
    output_consumer: Box<dyn OutputConsumer>,

    /// Builder used to create a new CSV writer for each received data
    /// buffer.
    builder: CsvWriterBuilder,
    config: CsvEncoderConfig,
    buffer: Vec<u8>,
    max_buffer_size: usize,
}

impl CsvEncoder {
    fn new(output_consumer: Box<dyn OutputConsumer>, config: CsvEncoderConfig) -> Self {
        let mut builder = CsvWriterBuilder::new();
        builder.has_headers(false);
        let max_buffer_size = output_consumer.max_buffer_size_bytes();

        Self {
            output_consumer,
            builder,
            config,
            buffer: Vec::new(),
            max_buffer_size,
        }
    }
}

impl Encoder for CsvEncoder {
    fn consumer(&mut self) -> &mut dyn OutputConsumer {
        self.output_consumer.as_mut()
    }

    fn encode(&mut self, batches: &[Arc<dyn SerBatch>]) -> AnyResult<()> {
        let buffer = take(&mut self.buffer);
        let mut writer = self.builder.from_writer(buffer);
        let mut num_records = 0;

        for batch in batches.iter() {
            let mut cursor = batch.cursor();

            while cursor.key_valid() {
                let prev_len = writer.get_ref().len();

                let w = cursor.weight();
                writer.serialize((cursor.key(), w))?;
                let _ = writer.flush();

                // Drop the last encoded record if it exceeds max_buffer_size.
                // The record will be included in the next buffer.
                let new_len = writer.get_ref().len();
                let overflow = if new_len > self.max_buffer_size {
                    if num_records == 0 {
                        let record = std::str::from_utf8(&writer.get_ref()[prev_len..new_len])
                            .unwrap_or_default();
                        // We should be able to fit at least one record in the buffer.
                        bail!("CSV record exceeds maximum buffer size supported by the output transport. Max supported buffer size is {} bytes, but the following record requires {} bytes: '{}'.",
                              self.max_buffer_size,
                              new_len - prev_len,
                              truncate_ellipse(record, MAX_RECORD_LEN_IN_ERRMSG, "..."));
                    }
                    true
                } else {
                    num_records += 1;
                    false
                };

                if num_records >= self.config.buffer_size_records || overflow {
                    let mut buffer = writer.into_inner()?;
                    if overflow {
                        buffer.truncate(prev_len);
                    }
                    // println!("push_buffer {}", buffer.len() /*std::str::from_utf8(&buffer).unwrap()*/);
                    self.output_consumer.push_buffer(&buffer);
                    buffer.clear();
                    num_records = 0;
                    writer = self.builder.from_writer(buffer);
                }

                if !overflow {
                    cursor.step_key();
                }
            }
        }

        let mut buffer = writer.into_inner()?;

        if num_records > 0 {
            self.output_consumer.push_buffer(&buffer);
            buffer.clear();
        }

        self.buffer = buffer;

        Ok(())
    }
}
