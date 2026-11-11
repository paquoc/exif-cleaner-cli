# Exif Cleaner PSD

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

## App Exif Cleaner PSD (Tauri 2 + Vite React)

```sh
cd app
npm install
npx tauri dev      # chạy thử
npx tauri build    # ra target/release/exif-cleaner-psd.exe và bộ cài target/release/bundle/nsis/
```

Thêm file/thư mục (nút, kéo thả, hoặc kéo file vào icon exe), bấm Chạy. Trước khi chạy app kiểm tra file còn tồn tại, file thiếu được báo và bỏ qua. Mỗi dòng hiện tiến độ riêng, nút Dừng ngừng sau bước hiện tại (file đang làm giữ nguyên). Panel Output: thư mục output hoặc cùng thư mục, giữ cấu trúc thư mục, file gốc vào Thùng rác hoặc xóa vĩnh viễn, hậu tố tên file. Cùng thư mục và không hậu tố nghĩa là ghi đè; app hỏi xác nhận trước khi ghi đè hoặc xóa vĩnh viễn.

## Cách hoạt động

- `crates/core`: đọc file theo section (header, resources, layer & mask, image data) và chỉ ghi lại resource XMP 1060 cùng các block `lnk2`/`lnk3`/`lnkD` chứa smart object đã đổi. Layer record, channel data và merged image được copy nguyên byte, không decode hay nén lại.
- XMP: định vị property theo namespace `http://ns.adobe.com/photoshop/1.0/` (roxmltree), rồi cắt đúng đoạn byte đó; metadata khác giữ nguyên.
- Smart object: `liFD` v1–8. Chỉ thay payload, data length, item length, và padding khi alignment đổi. UUID, tên, descriptor và các trường suffix (child id, mtime, lock, contentID, extension) giữ nguyên byte.
- Smart object trùng nội dung (blake3) chỉ clean một lần. Smart object không phải PSD/PSB (PNG, JPEG, AI, PDF...) và linked file ngoài (`lnkE`) để nguyên.
- File đã sạch sẵn: ghi đè thì không đụng tới; ghi ra đường dẫn mới thì copy nguyên.
- Trước khi ghi: parse lại toàn bộ cây output, yêu cầu không lỗi và không còn ancestors. Ghi qua file tạm rồi rename. Nếu file bị sửa trên đĩa trong lúc clean thì không ghi.

Cấu trúc tham khảo Adobe Photoshop File Formats Specification và photocraft-psd (MIT/Apache-2.0): danh sách key PSB 8-byte, cách dò padding của global block, layout `liFD`, các OSType của descriptor.

## Phát hành

Version nằm ở một chỗ duy nhất: `version` trong [Cargo.toml](Cargo.toml) (app, title cửa sổ và tên bộ cài đều lấy từ đây).

```sh
# sửa version trong Cargo.toml (và app/package.json cho khớp), commit, rồi:
git tag v1.0.0
git push origin v1.0.0
```

GitHub Actions ([.github/workflows/release.yml](.github/workflows/release.yml)) build trên Windows, kiểm tra tag khớp version, chạy test, rồi tạo release kèm bộ cài, bản portable và CLI. Chạy tay (Actions > Release > Run workflow) để build thử mà không publish; file nằm trong artifact của lần chạy đó.

### Tự cập nhật

Khi mở, app đọc `latest.json` của release mới nhất. Có bản mới thì hiện thanh thông báo: bản cài (NSIS) bấm "Update and restart" để tải bộ cài mới, kiểm chữ ký, cài đè rồi mở lại; bản portable chỉ có nút mở trang release. Đang chạy clean thì không cập nhật được.

CI ký bộ cài bằng khóa minisign và đính `.sig` + `latest.json` vào release ([tauri.release.conf.json](app/src-tauri/tauri.release.conf.json) bật `createUpdaterArtifacts`). Public key nằm trong [tauri.conf.json](app/src-tauri/tauri.conf.json) (`plugins.updater.pubkey`); private key cần ở secret `TAURI_SIGNING_PRIVATE_KEY` (và `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). Mất private key thì các bản đã cài không nhận update được nữa, phải cài lại tay. Build ở máy (`npx tauri build`) không cần khóa.
