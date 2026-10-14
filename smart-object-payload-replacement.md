# Spec: thay payload Smart Object nhúng, bảo toàn record

Ngày: 2026-10-14. Trạng thái: đề xuất, chưa triển khai.

## Mục tiêu

Bổ sung primitive Rust để thay nội dung file nguồn nhúng trong PSD/PSB mà không mở Photoshop, không import vào document model và không render lại ảnh. Đây là nền tảng cho cleaner xóa XMP `photoshop:DocumentAncestors` trong các Smart Object lồng nhau.

Scope của thay đổi này chỉ là thay payload; không triển khai cleaner, CLI, xử lý XML, duyệt thư mục hay ghi file ngoài.

## Nền hiện có

- `crates/psd/src/file.rs`: `PsdFile::from_bytes()` và `to_bytes()` đọc/ghi cấu trúc PSD/PSB, giữ dữ liệu pixel đã nén và block thô. Có một số trường hợp normalization; không mặc định coi mọi file round-trip đều byte-identical.
- `crates/io/src/linked.rs`: đọc các file `liFD` trong block `lnk2`, `lnk3`, `lnkD`; có `encode_linked_file()` và `rebuild_block()`.
- `encode_linked_file()` tạo record mới, version 7, với child document id, asset modification time và asset locked mặc định. Không dùng hàm này để thay record của cleaner.
- `TaggedBlock::check_structure()` có thể kiểm tra cấu trúc linked-layer, nhưng kiểm tra hiện tại không xác nhận đầy đủ mọi trường version-specific sau payload.

## API đề xuất

Đặt trong `crates/io/src/linked.rs`, không thêm dependency:

```rust
pub fn replace_embedded_payload(
    block_data: &[u8],
    uuid: &str,
    new_payload: &[u8],
) -> Result<Option<Vec<u8>>, crate::IoError>;
```

Input là **data của một linked-layer block**, không gồm signature/key/length/padding của tagged block ngoài. Caller chỉ gọi cho các key được hỗ trợ ở trên và byte order big-endian PSD; không áp dụng trực tiếp cho block TIFF có byte order khác.

- `Ok(Some(data))`: đúng một record được thay; trả data block mới.
- `Ok(None)`: tìm được đúng một record hợp lệ và payload đã bằng input mới; caller giữ nguyên bytes/Arc cũ.
- `Err`: UUID rỗng, không tìm thấy, UUID trùng, record đích không phải `liFD`, version không hỗ trợ, input hỏng hoặc không đủ bộ nhớ. Không dùng `None` cho trường hợp không tìm thấy.
- Không sửa buffer đầu vào và không có filesystem side effect. Khi lỗi, caller không nhận kết quả một phần.

## Quy tắc bảo toàn

Giữ nguyên thứ tự record và bytes của tất cả record không được thay. Record được thay phải giữ nguyên:

- kind `liFD`, version, UUID và tên file dưới dạng bytes gốc;
- file type, creator;
- cờ và bytes của optional open descriptor;
- phần sau payload: child document id, modification time, lock state và extension bytes;
- mọi dữ liệu pixel/composite, transform, filter và descriptor nằm ngoài payload.

Chỉ thay payload, trường `u64 data length`, trường `u64 item length` và padding ngoài item cần thiết cho alignment 4. Hai length đều big-endian; item length không gồm prefix 8 byte và padding.

Nếu số byte padding cần thiết không đổi, giữ padding gốc, kể cả padding khác zero. Nếu số byte padding thay đổi, sinh đúng số byte zero cần thiết. Payload không đổi thì toàn block giữ nguyên tuyệt đối.

File type không được tự suy ra lại. Caller phải cung cấp payload cùng định dạng nguồn; API này không phải convert Smart Object.

## Parsing và lỗi

1. Duyệt toàn bộ block để xác nhận framing và UUID duy nhất trước khi trả kết quả, kể cả khi payload bằng payload cũ.
2. UUID so sánh chính xác bằng UTF-8 hợp lệ, không dùng `String::from_utf8_lossy()` để xác định record. UUID không hợp lệ hoặc không thể xác định identity trong block là lỗi; tránh thay nhầm record.
3. Xác minh đủ prefix 8 byte, item length, range item và padding giữa các item. Bản đầu yêu cầu padding đầy đủ, kể cả item cuối; block có tail/truncation không xác minh được bị từ chối, không âm thầm bỏ dữ liệu.
4. Với record đích: chỉ hỗ trợ version 1–7; từ chối version khác thay vì đoán offset. Record không phải đích không cần diễn giải phần payload/version-specific khi framing và identity đã hợp lệ.
5. Đọc prefix của item để lấy offset trường data length, optional descriptor, start/end payload. Kiểm tra descriptor bằng parser hiện có nhưng giữ bytes gốc khi ghi.
6. Kiểm tra các trường suffix bắt buộc theo version: child document id từ v5 (Unicode string có length), modification time từ v6 (8 byte), lock state từ v7 (1 byte). Giữ nguyên bytes, kể cả extension sau các trường đã biết. Test generator phải tự dựng fixture đúng layout, không chỉ dựa vào `encode_linked_file()` hiện có.
7. Dùng `checked_add/sub`, conversion có kiểm tra và `get()` cho mọi range input. Tính kích thước output có kiểm tra; dùng `try_reserve` và chuyển allocation failure thành `Err`. Không decode pixel hoặc cấp phát theo kích thước ảnh khai báo.

Nên dùng helper nội bộ trả layout/offset đã xác minh, tránh tạo bản sao payload chỉ để xác định vị trí. Không thay đổi semantics các reader cũ trong cùng patch.

## Ghép vào PSD/PSB

Caller thay `TaggedBlock.data` bằng kết quả mới, giữ signature/key và để writer tính lại length ngoài theo PSD/PSB. Vì `TaggedBlock.padding` có thể lưu padding gốc tường minh, caller phải cập nhật padding ngoài khi độ dài data đổi: giữ bytes cũ nếu còn đúng alignment, nếu không dùng padding mặc định. Không chỉ thay `data` rồi giữ padding cũ một cách máy móc.

Caller lưu PSD/PSB qua `PsdFile::to_bytes()`, không qua `psd_to_document()`/`document_to_psd()`. Primitive không tự cập nhật cached render: tác vụ metadata-only không làm thay đổi pixels. Với chỉnh sửa nội dung ảnh, caller cần một pipeline khác để cập nhật cache và các thông tin liên quan.

UUID uniqueness của API chỉ áp dụng trong một block. Cleaner tương lai phải kiểm tra nguồn/UUID trên toàn document, xử lý các tham chiếu dùng chung nhất quán và giới hạn depth/tổng bytes khi đệ quy.

## Kiểm thử và tiêu chí nghiệm thu

- Fixture version 1–7; descriptor có/không; tên Unicode; suffix có giá trị khác mặc định và extension bytes.
- Thay payload nhỏ hơn, lớn hơn, rỗng, cùng chiều dài; bao phủ mọi residue modulo 4 của item length. Kiểm tra byte-for-byte prefix ngoài length, suffix và các record lân cận.
- Padding khác zero được giữ khi số byte padding không đổi; padding mới đúng khi alignment đổi. No-op trả `None`.
- UUID rỗng, không tồn tại, trùng, UTF-8 lỗi; đích `liFE`/`liFA`; version không hỗ trợ đều trả `Err`.
- Truncation sweep, length overflow, descriptor hỏng, suffix thiếu, padding thiếu và tail hỏng: không panic, không trả block đã sửa một phần.
- Tích hợp synthetic PSD và PSB: nguồn PSD/PSB nhỏ nằm trong Smart Object, thay payload rồi serialize/reparse; đọc lại đúng payload mới, giữ nguyên channel/composite bytes và các block không liên quan. Bao phủ outer tagged padding tường minh và block đứng trước một block khác.
- Chạy `TaggedBlock::check_structure()` trên output hợp lệ, nhưng không dùng nó làm oracle duy nhất cho suffix preservation.

Gate khi triển khai: `cargo test -p photocraft-io`, `cargo clippy -p photocraft-io --all-targets -- -D warnings`, `cargo xtask layers`, `cargo xtask wasm`, `cargo xtask test-corpus`; thêm devlog. Đây là library primitive, chưa thêm engine command nên chưa cần parity regeneration hoặc command panic_hunt.

## Cleaner tương lai (ngoài scope)

Đọc XMP resource 1060, xóa đúng property theo namespace, giữ metadata khác; đệ quy PSD/PSB nhúng rồi dùng primitive này ghép lên các cấp cha. Mặc định ghi output mới, có dry-run/thống kê và ghi file atomic. Linked Smart Object ngoài document và nguồn JPEG/TIFF cần đặc tả riêng. Không hứa mức giảm dung lượng trước khi đo trên file thực tế.
