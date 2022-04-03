//! Simple program to read an image and add the FLAC METADATA_BLOCK_PICTURE header
//! so it can be used as cover art in FLAC and OGG files.
//! This is mainly because ffmpeg doesn't support `-disposition:v attached_pic`
//! in ogg containers, even though it can for FLAC and it's the same format,
//! and it can decode this format for ogg containers.
//!
//! Outputs either raw binary form, base64-encoded, or as an ffmetadata ini file. An ffmpeg
//! metadata file is the default format.
//! Note: you can combine a metadata ini with additional metadata on the command line
//! and ffmpeg will merge them, e.g.
//! `ffmpeg -i input -i <(ogg-coverart -f cover.png) -map_metadata 1 -metadata title=Foo [...] output.ogg`
//!
//! See:
//! https://wiki.xiph.org/VorbisComment#METADATA_BLOCK_PICTURE
//! https://xiph.org/flac/format.html#metadata_block_picture
//!
//! Copyright 2020 Allen Wild <allenwild93@gmail.com>
//! SPDX-License-Identifier: Apache-2.0

use std::fs::{self, File};
use std::io::{self, BufWriter, Cursor, Write};

use anyhow::Result;
use clap::{crate_version, AppSettings, Arg, ArgGroup, Command};

/// Give writers a method to write big-endian u32 values
trait WriteU32 {
    fn write_u32b(&mut self, n: u32) -> io::Result<()>;
}

impl<W: Write> WriteU32 for W {
    fn write_u32b(&mut self, n: u32) -> io::Result<()> {
        let data = n.to_be_bytes();
        self.write_all(&data)
    }
}

/// Represents an image file and its extracted metadata needed to generate a METADATA_BLOCK_PICTURE
/// data structure.
#[derive(Debug)]
struct MetadataBlockPicture<'a> {
    mime: &'a str,
    width: u32,
    height: u32,
    bit_depth: u32,
    data: &'a [u8],
}

impl<'a> MetadataBlockPicture<'a> {
    /// Parse the info needed for picture metadata from the data of a PNG file.
    ///
    /// `data` argument is the full contents of a valid PNG image and will be borrowed for as long
    /// as this `MetadataBlockPicture` exists.
    fn from_png(data: &'a [u8]) -> Result<Self> {
        use png::{BitDepth, ColorType, Decoder};

        let decoder = Decoder::new(Cursor::new(data));
        let reader = decoder.read_info()?;
        let info = reader.info();

        // note: this ignores indexed color PNGs and treats them the same as a grayscale image in
        // the metadata, i.e. the index color count field is left at zero. Presumably this is fine
        // in practice. The xiph flac docs use GIF as an example of indexed color images, not PNG,
        // and I'm not quite sure how to get the pallette size properly.
        let num_channels = match info.color_type {
            ColorType::Grayscale => 1,
            ColorType::Rgb => 3,
            ColorType::Indexed => 1,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgba => 4,
        };

        let bits_per_channel = match info.bit_depth {
            BitDepth::One => 1,
            BitDepth::Two => 2,
            BitDepth::Four => 4,
            BitDepth::Eight => 8,
            BitDepth::Sixteen => 16,
        };

        let bit_depth = num_channels * bits_per_channel;

        Ok(Self {
            mime: "image/png",
            width: info.width,
            height: info.height,
            bit_depth,
            data,
        })
    }

    /// Write the METADATA_BLOCK_PICTURE header and data to the given writer
    fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        w.write_u32b(3)?; // type: cover(front)
        w.write_u32b(
            self.mime
                .len()
                .try_into()
                .expect("MIME type length overflow"),
        )?;
        w.write_all(self.mime.as_bytes())?;
        w.write_u32b(0)?; // description length
        w.write_u32b(self.width)?;
        w.write_u32b(self.height)?;
        w.write_u32b(self.bit_depth)?;
        w.write_u32b(0)?; // index color count, not used for png/jpg
        w.write_u32b(self.data.len().try_into().expect("data length overflow"))?;
        w.write_all(self.data)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Binary,
    Base64,
    FFMetadata,
}

fn run() -> Result<()> {
    let args = Command::new("ogg-coverart")
        .version(crate_version!())
        .about("Generate FLAC/OGG METADATA_BLOCK_PICTURE tag data from an image")
        .setting(AppSettings::DeriveDisplayOrder)
        .arg(Arg::new("input").required(true).help("Input image file"))
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .takes_value(true)
                .help("Output file, omit or use - for stdout"),
        )
        .arg(
            Arg::new("fmt_ffmetadata")
                .short('f')
                .long("ffmetadata")
                .help("Output in FFMETADATA1 INI format (default)"),
        )
        .arg(
            Arg::new("fmt_bin")
                .short('b')
                .long("binary")
                .help("Output in raw binary format"),
        )
        .arg(
            Arg::new("fmt_base64")
                .short('B')
                .long("base64")
                .help("Output in raw base64 format"),
        )
        .group(
            ArgGroup::new("format")
                .args(&["fmt_ffmetadata", "fmt_bin", "fmt_base64"])
                .multiple(false)
                .required(true),
        )
        .get_matches();

    let out_fmt = if args.is_present("fmt_bin") {
        OutputFormat::Binary
    } else if args.is_present("fmt_base64") {
        OutputFormat::Base64
    } else if args.is_present("fmt_ffmetadata") {
        OutputFormat::FFMetadata
    } else {
        unreachable!()
    };

    let data = fs::read(&args.value_of("input").unwrap())?;
    let meta = MetadataBlockPicture::from_png(&data)?;

    let mut out: Box<dyn Write> = match args.value_of("output") {
        None | Some("-") => Box::new(io::stdout()),
        Some(path) => Box::new(BufWriter::new(File::create(path)?)),
    };

    match out_fmt {
        OutputFormat::Binary => meta.write_to(&mut out)?,
        OutputFormat::Base64 | OutputFormat::FFMetadata => {
            // ffmetadata is just base64 but with an additional header prepended
            if out_fmt == OutputFormat::FFMetadata {
                out.write_all(b";FFMETADATA1\nMETADATA_BLOCK_PICTURE=")?;
            }
            let mut b64_out = base64::write::EncoderWriter::new(&mut out, base64::STANDARD);
            meta.write_to(&mut b64_out)?;
            b64_out.finish()?;
        }
    }

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
    }
}
