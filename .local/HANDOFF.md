# rustxl — Oturum Devir Notu

> **Tarih:** 2026-07-25 · **Depo:** `~/projects/rustxl` (origin=CyPack/rustxl, upstream=only-using-ai/rustxl)
> **Kanonik tasarım:** `.local/prd/2026-07-25-rustxl-multisheet-xlsx-write-design.md` — ÖNCE ONU OKU
> **Bağlam:** herdr FM Katman 2 (harici uygulamayı tab olarak barındırma) için düzenlenebilir tablo bileşeni

---

## Şu ana kadar TAMAMLANANLAR (hepsi commit'li + fork'a push'lu)

| Faz | Branch | Commit | Ne yapıldı | Kapı |
|---|---|---|---|---|
| **P0** | `feat/license` | `9b6fc0e` | MIT LICENSE + `Cargo.toml` license/description/repository/readme + README netleştirme | 63/63 · metadata `license="MIT"` · `cargo package` LICENSE'ı içeriyor |
| **P1.1** | `test/characterization` | `ac9c33b` | 11 karakterizasyon testi (6 yükleme + 5 kaydetme) + `tests/fixtures/three_sheets.xlsx` + fixture README | 74/74 · sadece EKLEME (0 silme) · yeni kod rustfmt-temiz |
| **P1.2** | `feat/positional-file` | `d4e2758` | `xl dosya.xlsx` pozisyonel argüman (+`conflicts_with`), README güncellemesi | 68/68 · pty E2E: pozisyonel ≡ `-f` (4793 kar birebir) |
| **P1.3** | `fix/remove-unwrap` | `88abe3c` | 27 üretim `unwrap()` → **0**; `strip_matching_quotes` + `palette_index` yardımcıları; `PROPER` çok-karakter hatası düzeltildi | 73/73 · clippy delta **0** · formula.rs rustfmt farkı 156→145 |

**Her branch `upstream/main`'den bağımsız dallanıyor** → PR'lar herhangi bir sırada merge edilebilir.
`planning` branch'i sadece `.local/` planlama dosyalarını taşır, **upstream'e ASLA gitmez**.

### P1.3'te bulunan ve düzeltilen gizli hata

`PROPER("ßeta")` → `"Seta"` üretiyordu. `char::to_uppercase()` çok karakter döndürebilir
(`ß`→`SS`), `.next()` ikincisini sessizce atıyordu. `extend` ile düzeltildi → `"SSeta"`.
Test: `formula::case_conversion::proper_keeps_every_character_of_a_multi_character_uppercase`.

---

## SIRADAKİ: P2.1 — `Sheet` struct'ını ayır (saf refactor)

**Branch:** `refactor/sheet-struct`, `upstream/main`'den + `test/characterization` merge edilerek
(karakterizasyon testleri güvenlik ağı olarak GEREKLİ).

```bash
cd ~/projects/rustxl
git checkout -b refactor/sheet-struct upstream/main
git merge --no-ff test/characterization   # güvenlik ağını al
```

**Yapılacak:** `Spreadsheet`'ten belge+görünüm alanlarını `Sheet`'e taşı:

```rust
struct Sheet {
    cells: HashMap<(usize, usize), String>,
    cell_styles: HashMap<(usize, usize), CellStyle>,
    col_widths: HashMap<usize, u16>,
    row_heights: HashMap<usize, u16>,
    num_rows: usize,
    num_cols: usize,
    cursor_row: usize,
    cursor_col: usize,
    scroll_row: usize,
    scroll_col: usize,
}

struct Spreadsheet {
    sheets: Vec<Sheet>,
    active_sheet: usize,
    // ... kalan uygulama state'i (mod bayrakları, find, command, update, clipboard)
}
```

**Ölçülmüş blast radius** (bu yüzden lokal ve yapılabilir):

| Alan | Erişim | Nerede |
|---|---|---|
| `.cells` | 29 | 28'i `spreadsheet.rs`, 1'i `save.rs` |
| `.cell_styles` | 28 | çoğu `style.rs` |
| `.num_rows`/`.num_cols` | 30/32 | |
| `.cursor_row`/`.cursor_col` | 67/67 | `ui.rs` + `input.rs` — en yaygın |
| `.scroll_row`/`.scroll_col` | 6/11 | |

**Strateji önerisi:** Önce `Spreadsheet` üzerinde delegasyon metotları ekle
(`fn cells(&self) -> &HashMap<...> { &self.sheets[self.active_sheet].cells }`), çağrı yerlerini
metotlara çevir, EN SON alanları taşı. Böylece her adım derlenir ve testler yeşil kalır.

**ÇIKIŞ KAPISI (mutlak):** 74/74 test **değişmeden** yeşil. Tek bir test bile düzenlenmesi
gerekiyorsa refactor saf değildir → geri al, yaklaşımı değiştir.

---

## Kalan faz zinciri

```
P2.1 Sheet struct  →  P2.2 tüm sayfaları yükle  →┬→ P2.3 sayfa gezinme UI →┬→ P2.4 imleç hafızası
                                                  │                        └→ P2.5 MOUSE (M1-M8)
                                                  └→ P3.1 umya geçişi → P3.2 kirli takip → P3.3 xlsx kaydet
                                                                                    ↓
                                              P2.4 + P2.5 + P3.3  →  P4 herdr entegrasyonu + fiziksel test
```

Her fazın test noktaları task listesinde ve tasarım dokümanı §5'te.

---

## Doğrulanmış teknik zemin (tekrar araştırma GEREKMİYOR)

| Konu | Sonuç | Kanıt |
|---|---|---|
| xlsx yazma kütüphanesi | **umya-spreadsheet 3.0.1**, MIT, MSRV 1.88 | `/tmp/umya-probe` round-trip: 3 sayfa + formül + çapraz-sayfa formül + bool + tarih + `yyyy-mm-dd` biçimi **%100 korundu** |
| Round-trip stratejisi | Yüklemede umya book'u bellekte tut + kirli hücre kümesi (snapshot semantiği; yeniden-okuma TOCTOU riskli) | tasarım §3.2 |
| Test altyapısı | `cargo nextest` kurulu; crate **binary-only** → entegrasyon testi crate'i import EDEMEZ, testler **inline** olmalı | `Cargo.toml`'da `[lib]` yok |
| Kalite kapısı | **DELTA** olmalı, mutlak değil | baseline: rustfmt ~230 fark (proje hiç fmt'lenmemiş), clippy 23 uyarı |
| pty test aracı | `.local/tools/tui_probe.py` — gerçek binary'yi pty'de çalıştırır, tuş gönderir, ekranı döndürür | P1.2'de kullanıldı; **TIOCSWINSZ şart**, yoksa ratatui boş çizer |
| Sayfa gezinme kısayolu | `Tab` MEŞGUL (visual mode). Adaylar: `gt`/`gT` (sc-im, vim-uyumlu — güçlü hipotez) | P2.3'te tuş haritası taraması ile kanıtlanacak |

---

## Bekleyen KULLANICI kararı (dışa dönük, benim yapmam uygun değil)

1. **Upstream issue:** LICENSE dosyası talebi. Taslak hazır (sohbet geçmişinde). `gh issue create --repo only-using-ai/rustxl`
2. **Upstream PR'lar:** 4 branch hazır ve bağımsız. Sıra önerisi: license → positional → characterization → unwrap.
   (license ilk, çünkü diğerlerinin hukuki zeminini kurar.)

**Yazar sinyali:** Chase Willden — issue #1'i çözmüş, PR #2'yi merge etmiş → PR kabul ediyor.
Ama son commit 2026-02-23, **5 aydır sessiz** → yanıt gecikebilir. Fork bağımsız kullanılabilir kalır.

---

## Ortam notları

- Rust 1.96.1, `cargo-nextest` kurulu, `export PATH="$HOME/.local/bin:$PATH"` gerekli
- Görsel/fiziksel test yığını **doğrulandı**: `ydotool` (PID 1285, socket hazır) +
  `~/projects/click-bridge/tools/portal-screenshot.py` (5760×2160 yakaladı) + Wayland
- herdr izole lab (P4 için hazır): server `/tmp/herdr-lab-run1/config/herdr-dev/herdr.sock`,
  plugin `/tmp/herdr-plugin-lab/xleak/herdr-plugin.toml` — canlı herdr'a (PID 16087) SIFIR temas
- Kıyas binary'leri: `/tmp/tui-demo/bin/{xleak,csvlens,rustxl-xl}`, veri `/tmp/tui-demo/data/`

*Son güncelleme: 2026-07-25, P1.3 kapanışı.*
