# ogg-coverart
Simple Rust program to generate FLAC and OGG METADATA_BLOCK_PICTURE tag data.

simple program to read an image and add the FLAC METADATA_BLOCK_PICTURE header
so it can be used as cover art in FLAC and OGG files.
This is mainly because ffmpeg doesn't support `-disposition:v attached_pic`
in ogg containers, even though it can for FLAC and it's the same format,
and it can decode this format for ogg containers.

Outputs either raw binary form, base64-encoded, or as an ffmetadata ini file.
Note: you can combine a metadata ini with additional metadata on the command line
and ffmpeg will merge them, e.g.
`ffmpeg -i input -i <(ogg-coverart -f ff cover.png) -map_metadata 1 -metadata title=Foo [...] output`

TODO:
 * add support directly to ffmpeg (libavformat)
 * add support for jpeg images, and possibly lift the RGB8 restriction on PNG
 * add a mode to call ffmpeg and embed the picture (probably easier than fully parsing/writing the whole ogg container)
