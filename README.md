# exif-cleaner

Xóa XMP `photoshop:DocumentAncestors` khỏi file PSD/PSB và mọi Smart Object PSD/PSB nhúng bên trong (đệ quy), không cần Photoshop. Thay thế cho `ExifCleanerPSD.jsx`.

```sh
cargo build --release
target/release/exif-cleaner -n  <file|folder>...   # dry-run: báo số ancestors và dung lượng sau khi clean
target/release/exif-cleaner     <file|folder>...   # ghi đè tại chỗ, bản gốc chuyển vào _exif_backup/ cạnh file
target/release/exif-cleaner -o OUT <file|folder>... # ghi bản sạch ra OUT/, giữ nguyên bản gốc
```

Tùy chọn: `--no-backup`, `--max-depth N` (mặc định 32), `-v` (in cây smart object), `--json`. Ctrl+C dừng sau bước hiện tại; file đang xử lý giữ nguyên.

## Cách hoạt động

- `crates/core`: đọc file theo section (header, resources, layer & mask, image data) và chỉ ghi lại resource XMP 1060 cùng các block `lnk2`/`lnk3`/`lnkD` chứa smart object đã đổi. Layer record, channel data và merged image được copy nguyên byte, không decode hay nén lại.
- XMP: định vị property theo namespace `http://ns.adobe.com/photoshop/1.0/` (roxmltree), rồi cắt đúng đoạn byte đó; metadata khác giữ nguyên.
- Smart object: `liFD` v1–7. Chỉ thay payload, data length, item length, và padding khi alignment đổi. UUID, tên, descriptor và các trường suffix (child id, mtime, lock, extension) giữ nguyên byte.
- Smart object trùng nội dung (blake3) chỉ clean một lần. Smart object không phải PSD/PSB (PNG, JPEG, AI, PDF...) và linked file ngoài (`lnkE`) để nguyên.
- Trước khi ghi: parse lại toàn bộ cây output, yêu cầu không lỗi và không còn ancestors. Ghi qua file tạm rồi rename. Nếu file bị sửa trên đĩa trong lúc clean thì không ghi.

Cấu trúc tham khảo Adobe Photoshop File Formats Specification và photocraft-psd (MIT/Apache-2.0): danh sách key PSB 8-byte, cách dò padding của global block, layout `liFD`, các OSType của descriptor.
