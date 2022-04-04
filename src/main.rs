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
use std::path::Path;

use anyhow::{bail, Context, Result};
use clap::{crate_version, AppSettings, Arg, ArgGroup, ArgMatches, Command};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageType {
    Png,
    Jpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Binary,
    Base64,
    FFMetadata,
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
        let reader = decoder.read_info().context("PNG image parse")?;
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

    /// Parse the info needed for picture metadata from the data of a jpeg image file.
    fn from_jpeg(data: &'a [u8]) -> Result<Self> {
        use jpeg_decoder::{Decoder, PixelFormat};

        let mut decoder = Decoder::new(Cursor::new(data));
        decoder.read_info().context("JPG image parse")?;
        let info = decoder.info().unwrap();

        let bit_depth = match info.pixel_format {
            PixelFormat::L8 => 8,
            PixelFormat::L16 => 16,
            PixelFormat::RGB24 => 24,
            PixelFormat::CMYK32 => 32,
        };

        Ok(Self {
            mime: "image/jpeg",
            width: info.width.into(),
            height: info.height.into(),
            bit_depth,
            data,
        })
    }

    /// Parse the info needed for picture metadata using one of the supported types.
    fn from_type(data: &'a [u8], image_type: ImageType) -> Result<Self> {
        match image_type {
            ImageType::Png => Self::from_png(data),
            ImageType::Jpeg => Self::from_jpeg(data),
        }
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

fn parse_args() -> ArgMatches {
    Command::new("ogg-coverart")
        .version(crate_version!())
        .about("Generate FLAC/OGG METADATA_BLOCK_PICTURE tag data from an image")
        .override_usage("ogg-coverart [OPTIONS] {-f | -b | -B} [-o OUTPUT] INPUT")
        .setting(AppSettings::DeriveDisplayOrder)
        .arg(
            Arg::new("input")
                .required(true)
                .allow_invalid_utf8(true)
                .value_name("INPUT")
                .help("Input image file"),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .takes_value(true)
                .value_name("OUTPUT")
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
        .arg(
            Arg::new("force_png")
                .short('p')
                .long("png")
                .help("treat the input as a PNG image (overrides file extension autodetection)"),
        )
        .arg(
            Arg::new("force_jpeg")
                .short('j')
                .long("jpeg")
                .help("treat the input as a JPEG image (overrides file extension autodetection)"),
        )
        .group(
            ArgGroup::new("input_format")
                .args(&["force_png", "force_jpeg"])
                .multiple(false)
                .required(false),
        )
        .get_matches()
}

fn run() -> Result<()> {
    let args = parse_args();

    let input_path = Path::new(args.value_of_os("input").unwrap());
    let input_type = if args.is_present("force_png") {
        ImageType::Png
    } else if args.is_present("force_jpeg") {
        ImageType::Jpeg
    } else {
        match input_path.extension().and_then(std::ffi::OsStr::to_str) {
            Some("png") => ImageType::Png,
            Some("jpg" | "jpeg") => ImageType::Jpeg,
            _ => bail!(
                "can't determine image type (missing or unrecognized file extension)\n\
                 Use the --png or --jpeg flag to manually set the image format"
            ),
        }
    };

    let out_fmt = if args.is_present("fmt_bin") {
        OutputFormat::Binary
    } else if args.is_present("fmt_base64") {
        OutputFormat::Base64
    } else if args.is_present("fmt_ffmetadata") {
        OutputFormat::FFMetadata
    } else {
        unreachable!()
    };

    let data = fs::read(input_path).context("failed reading input file")?;
    let meta =
        MetadataBlockPicture::from_type(&data, input_type).context("failed to parse input file")?;

    let mut out: Box<dyn Write> = match args.value_of("output") {
        None | Some("-") => Box::new(io::stdout()),
        Some(path) => Box::new(BufWriter::new(
            File::create(path).context("failed to create output file")?,
        )),
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
            drop(b64_out);
            out.write_all(b"\n")?;
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
