export async function read_image(file) {
  const url = URL.createObjectURL(file);
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const maxSide = 1800;
    const ratio = Math.min(1, maxSide / Math.max(image.naturalWidth, image.naturalHeight));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(image.naturalWidth * ratio));
    canvas.height = Math.max(1, Math.round(image.naturalHeight * ratio));
    const context = canvas.getContext('2d');
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let type = 'image/jpeg';
    for (let i = 3; i < pixels.length; i += 4) {
      if (pixels[i] < 255) {
        type = 'image/png';
        break;
      }
    }
    return type === 'image/png' ? canvas.toDataURL(type) : canvas.toDataURL(type, 0.88);
  } finally {
    URL.revokeObjectURL(url);
  }
}
