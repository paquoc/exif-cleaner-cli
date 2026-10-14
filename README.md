# exif-cleaner

Xóa XMP `photoshop:DocumentAncestors` khỏi file PSD/PSB và mọi Smart Object PSD/PSB nhúng bên trong (đệ quy), không cần Photoshop. Thay thế cho `ExifCleanerPSD.jsx`.

## CLI

```sh
cargo build --release
target/release/exif-cleaner <file|folder>...                # ghi đè tại chỗ
target/release/exif-cleaner in.psd -o out/new_name.psd      # một file vào -> -o là đường dẫn file mới
target/release/exif-cleaner folder1 a.psd -o OUT            # nhiều file / thư mục -> -o là thư mục (giữ cấu trúc con)
target/release/exif-cleaner -n <file|folder>...             # dry-run: chỉ báo, không ghi gì
```

Tùy chọn: `--max-depth N` (mặc định 32), `-v` (in cây smart object), `--json`. Ctrl+C dừng sau bước hiện tại; file đang xử lý giữ nguyên.

## App (Tauri 2 + Vite React)

```sh
cd app
npm install
npx tauri dev      # chạy thử
npx tauri build    # ra target/release/exif-cleaner-app.exe và bộ cài target/release/bundle/nsis/
```

Thêm file/thư mục (nút, kéo thả, hoặc kéo file vào icon exe), bấm Chạy. Trước khi chạy app kiểm tra file còn tồn tại, file thiếu được báo và bỏ qua. Mỗi dòng hiện tiến độ riêng, nút Dừng ngừng sau bước hiện tại (file đang làm giữ nguyên). Panel Output: thư mục output hoặc cùng thư mục, giữ cấu trúc thư mục, chuyển file gốc vào Thùng rác hoặc `_exif_backup/`, hậu tố tên file. Cùng thư mục và không hậu tố nghĩa là ghi đè (app hỏi xác nhận nếu không chuyển file gốc đi).

## Cách hoạt động

- `crates/core`: đọc file theo section (header, resources, layer & mask, image data) và chỉ ghi lại resource XMP 1060 cùng các block `lnk2`/`lnk3`/`lnkD` chứa smart object đã đổi. Layer record, channel data và merged image được copy nguyên byte, không decode hay nén lại.
- XMP: định vị property theo namespace `http://ns.adobe.com/photoshop/1.0/` (roxmltree), rồi cắt đúng đoạn byte đó; metadata khác giữ nguyên.
- Smart object: `liFD` v1–7. Chỉ thay payload, data length, item length, và padding khi alignment đổi. UUID, tên, descriptor và các trường suffix (child id, mtime, lock, extension) giữ nguyên byte.
- Smart object trùng nội dung (blake3) chỉ clean một lần. Smart object không phải PSD/PSB (PNG, JPEG, AI, PDF...) và linked file ngoài (`lnkE`) để nguyên.
- File đã sạch sẵn: ghi đè thì không đụng tới; ghi ra đường dẫn mới thì copy nguyên.
- Trước khi ghi: parse lại toàn bộ cây output, yêu cầu không lỗi và không còn ancestors. Ghi qua file tạm rồi rename. Nếu file bị sửa trên đĩa trong lúc clean thì không ghi.

Cấu trúc tham khảo Adobe Photoshop File Formats Specification và photocraft-psd (MIT/Apache-2.0): danh sách key PSB 8-byte, cách dò padding của global block, layout `liFD`, các OSType của descriptor.
