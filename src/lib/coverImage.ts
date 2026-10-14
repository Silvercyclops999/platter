/**
 * A chosen cover image, ready to send: scaled down to at most 1000 px on
 * its long side and saved as JPEG, so the upload stays small. Where the
 * browser can't decode it (tests), the file goes as it is.
 */
export const COVER_MAX_PX = 1000;
const TYPES = ["image/jpeg", "image/png", "image/webp"];
const MAX_BYTES = 10 * 1024 * 1024;

export async function coverImage(file: File): Promise<{ bytes: Uint8Array; type: string }> {
  if (!TYPES.includes(file.type)) throw new Error("Choose a JPEG, PNG or WebP image.");
  if (file.size > MAX_BYTES) throw new Error("Choose an image under 10 MB.");
  const raw = async () => ({ bytes: new Uint8Array(await file.arrayBuffer()), type: file.type });
  if (typeof createImageBitmap !== "function" || typeof document === "undefined") return raw();
  try {
    const bmp = await createImageBitmap(file);
    const scale = Math.min(1, COVER_MAX_PX / Math.max(bmp.width, bmp.height));
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(bmp.width * scale));
    canvas.height = Math.max(1, Math.round(bmp.height * scale));
    const ctx = canvas.getContext("2d");
    if (!ctx) return raw();
    ctx.drawImage(bmp, 0, 0, canvas.width, canvas.height);
    bmp.close?.();
    const blob = await new Promise<Blob | null>((res) => canvas.toBlob(res, "image/jpeg", 0.9));
    if (!blob) return raw();
    return { bytes: new Uint8Array(await blob.arrayBuffer()), type: "image/jpeg" };
  } catch {
    return raw();
  }
}
