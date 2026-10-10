use super::{files, io_error, optional_bool, optional_str, optional_u64, required_str};
use image::{ImageFormat, ImageReader, Rgba, RgbaImage};
use serde_json::{Value, json};
use std::path::Path;

fn read(path: &Path) -> Result<RgbaImage, String> {
    let meta = std::fs::metadata(path).map_err(|e| io_error("Cannot inspect image", e))?;
    if !meta.is_file() || meta.len() > 64 * 1024 * 1024 {
        return Err("Image input must be a file of at most 64 MiB.".into());
    }
    let mut reader = ImageReader::open(path)
        .map_err(|e| io_error("Cannot open image", e))?
        .with_guessed_format()
        .map_err(|e| io_error("Cannot detect image format", e))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|e| io_error("Cannot decode image header", e))?;
    use image::ImageDecoder;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > 16 * 1024 * 1024 {
        return Err("Image exceeds 16 megapixels.".into());
    }
    image::DynamicImage::from_decoder(decoder)
        .map(|img| img.to_rgba8())
        .map_err(|e| io_error("Cannot decode image", e))
}

pub fn compare(args: &Value) -> Result<Value, String> {
    let left_path = Path::new(required_str(args, "leftPath")?);
    let right_path = Path::new(required_str(args, "rightPath")?);
    let output = optional_str(args, "outputPath")?.map(Path::new);
    let overwrite = optional_bool(args, "overwrite", false)?;
    let threshold = optional_u64(args, "threshold", 0, 255)? as u8;
    if let Some(output) = output {
        if output.exists() && !overwrite {
            return Err("Output already exists.".into());
        }
        let absolute =
            std::path::absolute(output).map_err(|e| io_error("Cannot resolve output", e))?;
        let resolved = output.canonicalize().unwrap_or(absolute);
        if resolved
            == left_path
                .canonicalize()
                .map_err(|e| io_error("Cannot resolve left image", e))?
            || resolved
                == right_path
                    .canonicalize()
                    .map_err(|e| io_error("Cannot resolve right image", e))?
        {
            return Err("Difference output must not replace an input image.".into());
        }
    }
    let left = read(left_path)?;
    let right = read(right_path)?;
    if left.dimensions() != right.dimensions() {
        return Err("Images must have equal dimensions.".into());
    }
    let (width, height) = left.dimensions();
    let total = u64::from(width) * u64::from(height);
    let mut different = 0u64;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (width, height, 0, 0);
    let mut mask = output.map(|_| RgbaImage::new(width, height));
    let mut max_difference = 0u8;
    for (index, (a, b)) in left.pixels().zip(right.pixels()).enumerate() {
        let delta =
            a.0.iter()
                .zip(b.0)
                .map(|(a, b)| a.abs_diff(b))
                .max()
                .unwrap_or(0);
        max_difference = max_difference.max(delta);
        if delta > threshold {
            different += 1;
            let x = index as u32 % width;
            let y = index as u32 / width;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            if let Some(mask) = mask.as_mut() {
                mask.put_pixel(x, y, Rgba([255, 0, 255, 255]));
            }
        }
    }
    if let (Some(output), Some(mask)) = (output, mask) {
        let mut encoded = std::io::Cursor::new(Vec::new());
        mask.write_to(&mut encoded, ImageFormat::Png)
            .map_err(|e| io_error("Cannot encode difference mask", e))?;
        files::persist_bytes(output, &encoded.into_inner(), overwrite)?;
    }
    let bounding = if different > 0 {
        json!({"x":min_x,"y":min_y,"width":max_x-min_x+1,"height":max_y-min_y+1})
    } else {
        Value::Null
    };
    Ok(
        json!({"width":width,"height":height,"totalPixels":total,"differentPixels":different,
        "differentPercent":if total>0 {different as f64/total as f64*100.0}else{0.0},
        "equal":different==0,"threshold":threshold,"maxChannelDifference":max_difference,
        "boundingBox":bounding,"outputPath":output.map(|p|p.to_string_lossy())}),
    )
}
