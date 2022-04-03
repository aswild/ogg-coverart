//! simple program to read an image and add the FLAC METADATA_BLOCK_PICTURE header
//! so it can be used as cover art in FLAC and OGG files.
//! This is mainly because ffmpeg doesn't support `-disposition:v attached_pic`
//! in ogg containers, even though it can for FLAC and it's the same format,
//! and it can decode this format for ogg containers.
//!
//! Outputs either raw binary form, base64-encoded, or as an ffmetadata ini file.
//! Note: you can combine a metadata ini with additional metadata on the command line
//! and ffmpeg will merge them, e.g.
//! `ffmpeg -i input -i <(ogg-coverart -f ff cover.png) -map_metadata 1 -metadata title=Foo [...] output`
//!
//! See:
//! https://wiki.xiph.org/VorbisComment#METADATA_BLOCK_PICTURE
//! https://xiph.org/flac/format.html#metadata_block_picture
//!
//! TODO: implement this in ffmpeg itself
//! TODO: add jpeg support with https://crates.io/crates/jpeg-decoder
//!
//! Copyright 2020 Allen Wild <allenwild93@gmail.com>
//! SPDX-License-Identifier: Apache-2.0

use std::fs::{self, File};
use std::io::prelude::*;
use std::io::{self, Cursor};

use anyhow::{anyhow, Result};
use base64::display::Base64Display;
use clap::{crate_version, Command, Arg};

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

#[derive(Debug)]
struct PicInfo {
    mime: String,
    width: u32,
    height: u32,
    depth: u32,
}

fn pic_info(data: &[u8]) -> Result<PicInfo> {
    let decoder = png::Decoder::new(Cursor::new(data));
    let reader = decoder.read_info()?;
    let info = reader.info();

    // these restrictions might not be necessary
    if info.color_type != png::ColorType::Rgb {
        return Err(anyhow!("PNG isn't RGB"));
    }
    if info.bit_depth != png::BitDepth::Eight {
        return Err(anyhow!("PNG isn't 8-bit"));
    }

    Ok(PicInfo {
        mime: "image/png".to_owned(),
        width: info.width,
        height: info.height,
        depth: 24, // 8-bits for 3 channels
    })
}

fn generate_pic_data(data: &[u8]) -> Result<Vec<u8>> {
    let info = pic_info(data)?;
    let mut out = Vec::<u8>::with_capacity(data.len() + (4 * 8) + info.mime.len());
    out.write_u32b(3)?; // type: cover(front)
    out.write_u32b(info.mime.len() as u32)?;
    out.write_all(info.mime.as_bytes())?;
    out.write_u32b(0)?; // description length
    out.write_u32b(info.width)?;
    out.write_u32b(info.height)?;
    out.write_u32b(info.depth)?;
    out.write_u32b(0)?; // index color count, not used for png/jpg
    out.write_u32b(data.len() as u32)?;
    out.write_all(data)?;
    Ok(out)
}

fn main() -> Result<()> {
    let args = Command::new("ogg-coverart")
        .version(crate_version!())
        .about("Generate FLAC/OGG METADATA_BLOCK_PICTURE tag data from an image")
        .arg(
            Arg::new("format")
                .short('f')
                .long("format")
                .takes_value(true)
                .possible_values(&["bin", "b64", "ff"])
                .help("output format: binary, base64 encoded, or ffmetadata.ini"),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .takes_value(true)
                .help("output file, omit or use - for stdout"),
        )
        .arg(
            Arg::new("input")
                .required(true)
                .help("Input image file"),
        )
        .get_matches();

    let file_data = fs::read(args.value_of("input").unwrap())?;
    let pic_data = generate_pic_data(&file_data)?;
    let pic_b64 = Base64Display::with_config(&pic_data, base64::STANDARD);

    let mut out: Box<dyn Write> = match args.value_of("output") {
        None | Some("-") => Box::new(io::stdout()),
        Some(path) => Box::new(File::create(path)?),
    };

    match args.value_of("format") {
        None | Some("bin") => out.write_all(&pic_data)?,
        Some("b64") => writeln!(out, "{}", pic_b64)?,
        Some("ff") => writeln!(out, ";FFMETADATA1\nMETADATA_BLOCK_PICTURE={}", pic_b64)?,
        Some(_) => unreachable!(),
    };

    Ok(())
}
