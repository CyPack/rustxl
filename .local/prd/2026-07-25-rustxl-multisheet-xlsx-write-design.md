# rustxl — Çok Sayfa + xlsx Yazma: Tasarım Dokümanı

> **Durum:** Taslak → uygulama için hazır
> **Tarih:** 2026-07-25
> **Hedef depo:** `only-using-ai/rustxl` (upstream) · `CyPack/rustxl` (fork)
> **Çalışma alanı:** `~/projects/rustxl`
> **İlgili:** herdr FM Katman 2 (harici uygulamayı tab olarak barındırma) — plugin deneyi kanıtlandı

---

## 1. Bağlam ve Amaç

### 1.1 Neden bu proje?

herdr (terminal agent runtime) için bir dosya yöneticisi geliştiriliyor. Gezinirken tablo
dosyalarını (xlsx/csv) **görüntülemek** ve **düzenlemek** gerekiyor. Mimari karar (kanıtlanmış,
`PluginPaneOpen` deneyi ile): herdr bu uygulamaları **gömmez**, plugin paneli olarak **barındırır**.

Görüntüleme tarafı çözülü (xleak, MIT, çok sayfa). **Düzenleme tarafı boş.** Ölçülen aday envanteri:

| Aday | Düzenleme | xlsx yaz | Çok sayfa | Lisans | Engel |
|---|---|---|---|---|---|
| xleak | ❌ | ❌ | ✅ | MIT | editör değil, tasarım gereği |
| csvlens | ❌ | ❌ | — | MIT | sadece csv görüntüleyici |
| visidata | ✅ | ✅ | ✅ | GPL-3 | Python bağımlılığı |
| sc-im | ✅ | ⚠️ derlemeye bağlı | ✅ | BSD | Fedora paketi xlsx'siz derlenmiş |
| **rustxl** | ✅ | ❌ | ❌ | **beyan MIT, dosya yok** | aşağıdaki boşluklar |

rustxl **bugün düzenleme yapabilen tek Rust projesi**. Boşlukları kapatılırsa herdr'ın
düzenleme katmanı için doğal aday olur: aynı dil, aynı araç zinciri (cargo/nextest/clippy),
izin verici lisans hedefi.

### 1.2 İş hedefi

rustxl'i, çok sayfalı bir xlsx dosyasını **açıp düzenleyip kaybetmeden geri yazabilen** bir
terminal editörüne dönüştürmek — ve bu katkıları **upstream'e PR olarak** vermek.

### 1.3 Kapsam dışı (YAGNI)

- Formül motorunu genişletmek (mevcut motor korunur, dokunulmaz)
- Grafik/pivot/koşullu biçimlendirme desteği
- `.ods` desteği
- Hücre stillerini düzenleme UI'ı (mevcut stil sistemi korunur)
- Performans optimizasyonu (ölçüm olmadan optimizasyon yapılmaz)

---

## 2. Mevcut Durum Analizi (ölçülmüş, tahmin değil)

### 2.1 Kod tabanı

| Metrik | Değer | Kaynak |
|---|---|---|
| Rust satır | 7.200 (11 dosya) | `wc -l src/*.rs` |
| En büyük dosya | `formula.rs` 2.586 | aynı |
| Test | **63/63 yeşil, 0.061s** | `cargo nextest run` |
| `unsafe` | 0 | `grep -rn unsafe src/` |
| `unwrap()` | 38 (formula 26, spreadsheet 7, input 2, main/settings/update 1'er) | `grep -c` |
| Test altyapısı | `cargo-nextest` kurulu, `#[cfg(test)] mod tests` yerleşik | koşum çıktısı |

### 2.2 Mimari: `Spreadsheet` bir god-object

`src/spreadsheet.rs:20` — 40+ alanlı tek struct, dört farklı sorumluluğu karıştırıyor:

| Sorumluluk | Alanlar |
|---|---|
| **Belge verisi** (sayfa başına olmalı) | `cells`, `cell_styles`, `col_widths`, `row_heights`, `num_rows`, `num_cols` |
| **Görünüm** (sayfa başına olmalı) | `cursor_row/col`, `scroll_row/col` |
| **Düzenleme state'i** | `editing`, `edit_buffer`, `formula_mode`, `selecting_ref`, `ref_*` |
| **Uygulama state'i** (global) | `save_mode`, `open_mode`, `find_*`, `command_*`, `update_*`, `dark_mode`, `visual_*` |

### 2.3 Blast radius ölçümü (refactor maliyetinin gerçek göstergesi)

```
.cells        29 erişim  → 28'i spreadsheet.rs içinde, 1'i save.rs
.cell_styles  28
.num_rows     30    .num_cols   32
.cursor_row   67    .cursor_col 67    (ui.rs + input.rs'e yayılmış)
.scroll_row    6    .scroll_col 11
```

**Sonuç:** Belge verisi metotların (`get_cell`, `set_cell`, `get_data_bounds`…) arkasında iyi
kapsüllenmiş. `cursor/scroll` daha yaygın ama mekanik. Refactor **lokal ve düşük riskli**.

### 2.4 Tespit edilen boşluklar

| # | Boşluk | Kanıt | Etki |
|---|---|---|---|
| G1 | LICENSE dosyası yok | `ls`, `Cargo.toml`, README:13 MIT rozeti + README:294 "See the LICENSE file" | **Hukuki blocker** |
| G2 | Sadece ilk sayfa okunuyor | `spreadsheet.rs:1206-1216` — `sheet_names()` alınıp `[0]` kullanılıyor, gerisi atılıyor | Veri kaybı riski |
| G3 | xlsx'e yazamıyor | `save.rs:41-50` — sadece `Csv`/`Tsv` | Round-trip imkânsız |
| G4 | 38 `unwrap()` | `grep` | Panic riski |
| G5 | Pozisyonel arg yok | `xl dosya.xlsx` → "unexpected argument" | UX + entegrasyon sürtünmesi |
| G6 | **Mouse yakalanıyor ama işlenmiyor** | `main.rs:136` `EnableMouseCapture` açık; `input.rs:44` sadece `Event::Key` eşleştiriyor | **Gerileme** (aşağı bak) |

**G2+G3 birlikte veri kaybı senaryosu üretir:** kullanıcı 3 sayfalı xlsx'i açar, ilk sayfayı
görür, düzenler, kaydeder → elinde tek sayfalık CSV kalır, diğer 2 sayfa sessizce yok olur.

**G6 eksik özellik değil, gerileme:** `EnableMouseCapture` terminalin doğal fare davranışını
(metin seçip kopyalama, tekerlekle kaydırma) devre dışı bırakır. rustxl bunu kapatıyor ama
karşılığında hiçbir fare etkileşimi sunmuyor. Yani kullanıcı, uygulamayı açtığı anda sahip
olduğu bir yeteneği **kaybediyor**. İki geçerli çözüm var: (a) yakalamayı kapatmak (yetenek
geri gelir, fare özelliği olmaz), (b) olayları işlemek. **(b) seçildi** — çünkü hedef kullanım
(herdr içinde tab olarak gömülü tablo editörü) fare etkileşiminden doğrudan fayda görür.

---

## 3. Teknik Kararlar

### 3.1 xlsx yazma kütüphanesi: `umya-spreadsheet` (KARAR)

| Aday | Oku | Yaz | Round-trip sadakati | Lisans | Ağırlık |
|---|---|---|---|---|---|
| calamine (mevcut) | ✅ | ❌ | — | MIT | hafif |
| rust_xlsxwriter | ❌ | ✅ | **yok** (sıfırdan yazar, stil/diğer sayfa kaybolur) | MIT/Apache | 989 KB |
| **umya-spreadsheet** | ✅ | ✅ | **%100 (ampirik)** | **MIT** | 8.2 MB, 86 transitive |

**Ampirik doğrulama** (`/tmp/umya-probe`, 3 sayfalı gerçek fixture ile):

```
okundu: 3 sayfa [Sales, Inventory, Summary]
O2 formül : "SUM(C2:N2)"          ← formül metni okunabiliyor
A2 değiştirildi → yazıldı → openpyxl ile doğrulandı:
  sayfalar        ✅ 3'ü de, boyutlar birebir
  formül          ✅ =SUM(C2:N2), =AVERAGE(C2:N2)
  çapraz-sayfa    ✅ =SUM(Sales!O2:O31)
  bool tipi       ✅ True (bool olarak)
  tarih + biçim   ✅ datetime + yyyy-mm-dd
```

**Karar gerekçesi:** Round-trip sadakati bu projenin *tek kritik gereksinimi*. Kullanıcının
dokunmadığı veriyi kaybetmek kabul edilemez bir hata sınıfı. `rust_xlsxwriter` bunu yapısal
olarak sağlayamaz (yazma-only, kaynak dosyayı bilmez).

**Bağımlılık maliyeti dengelemesi:** umya okuma da yaptığı için `calamine` **kaldırılabilir**.
Net artış, iki kütüphaneyi yan yana tutmaktan düşük. (`.xls` eski format desteği kaybı → §6 risk.)

### 3.2 Round-trip stratejisi: bellekte workbook + kirli hücre takibi (KARAR)

İki seçenek değerlendirildi:

| Yaklaşım | Artı | Eksi |
|---|---|---|
| Kaydetmede kaynağı yeniden oku, diff uygula | Düşük bellek | **TOCTOU**: dosya diskte değiştiyse yanlış tabana yazar |
| **Yüklemede workbook'u bellekte tut** | **Snapshot semantiği**, öngörülebilir | Büyük dosyada bellek |

**Karar: bellekte tut.** Gerekçe: rustxl zaten tüm hücreleri `HashMap`'e yüklüyor — bellek
profili hâlihazırda "tam dosya". Snapshot semantiği veri güvenliği için daha savunulabilir.

```rust
// Kavramsal model
struct Spreadsheet {
    sheets: Vec<Sheet>,               // görünen/düzenlenen ızgara
    active_sheet: usize,
    source: Option<SourceWorkbook>,   // umya book + yol  (xlsx'ten açıldıysa)
    dirty: HashSet<(usize, usize, usize)>,  // (sheet_idx, row, col)
    // ... uygulama state'i
}
```

Kaydetmede: `dirty` kümesindeki hücreler book'a yazılır → `writer::xlsx::write`. Dokunulmayan
her şey (stiller, formüller, diğer sayfalar, biçimler) **hiç ellenmediği için** korunur.

### 3.3 Formül hücresi düzenleme semantiği (KARAR + BELGELENEN KAYIP)

Kullanıcı formül içeren bir hücreyi düzenlerse formül kaybolur, yerine girilen değer/formül gelir.
Bu **kaçınılmaz ve doğru** davranıştır (Excel de aynısını yapar). Ama sessiz olmamalı:
düzenleme başlarken hücrede formül varsa `edit_buffer`'a **ham formül** yüklenir (değer değil),
böylece kullanıcı ne değiştirdiğini görür.

### 3.4 Sayfa gezinme kısayolu (ARAŞTIRILACAK — P2.3'te karara bağlanacak)

`Tab` tuşu rustxl'de **zaten visual mode'a giriyor** (`input.rs`). Çakışma var. Emsaller:

| Uygulama | Kısayol |
|---|---|
| xleak | `Tab` / `Shift+Tab` |
| sc-im | `gt` / `gT` (vim tarzı) |
| Excel | `Ctrl+PageUp/PageDown` |

rustxl vim-tarzı bir uygulama (`:` komut modu, `gg`/`G` var) → **`gt`/`gT` hipotezi güçlü**,
ama P2.3'te mevcut tuş haritası taranarak kanıtlanacak. Karar test noktası ile birlikte verilecek.

### 3.5 Mouse: saf hit-testing + mod-farkında yönlendirme (KARAR)

Izgara geometrisi **değişken**: `col_widths: HashMap<usize, u16>` ve `row_heights` sabit değil,
`visible_cols(width)` hangi sütunların sığdığını zaten hesaplıyor. Dolayısıyla ekran koordinatını
hücreye çevirmek gerçek bir hesap — ve **saf bir fonksiyon** olarak yazılabilir:

```rust
enum HitTarget {
    Cell { row: usize, col: usize },
    ColumnHeader(usize),
    RowHeader(usize),
    SheetTab(usize),
    Outside,
}

fn hit_test(x: u16, y: u16, layout: &GridLayout) -> HitTarget  // saf, I/O yok, tablo-test edilebilir
```

**Neden saf fonksiyon:** fare davranışının doğruluğu koordinat matematiğinde yaşar (kaydırma
ofseti, değişken genişlik, başlık bölgeleri). Bunu render'dan ayırmak, ekran açmadan tablo
testiyle doğrulamayı mümkün kılar — asıl regresyon riski oradadır.

**Mod-farkında yönlendirme (fail-closed):** save/open/command/update modalleri açıkken ızgara
tıkları **yok sayılır**. Varsayılan "yok say"dır; bir mod tıkı açıkça talep etmelidir. Gerekçe:
modal açıkken arkadaki ızgarayı değiştiren bir tık, kullanıcının görmediği bir durum değişikliği
üretir — sessiz veri değişikliği en kötü hata sınıfıdır.

---

## 4. Faz Yapısı ve Bağımlılık Zinciri

```
P0  MIT LICENSE  (hukuki temel — her şeyin önkoşulu)
 │
 ├─→ P1.2 Pozisyonel arg          (bağımsız, küçük)
 ├─→ P1.3 unwrap temizliği        (bağımsız, mekanik)
 │
 └─→ P1.1 Karakterizasyon testleri  (güvenlik ağı)
      │
      └─→ P2.1 Sheet struct ayrımı  (saf refactor, davranış DEĞİŞMEZ)
           │
           └─→ P2.2 Tüm sayfaları yükle
                │
                ├─→ P2.3 Sayfa gezinme UI
                │    ├─→ P2.4 Sayfa başına imleç hafızası
                │    └─→ P2.5 MOUSE desteği  (M1..M8)
                │
                └─→ P3.1 umya'ya geçiş
                     └─→ P3.2 Kirli hücre takibi
                          └─→ P3.3 SaveFormat::Xlsx
                                          │
        P2.4 + P2.5 + P3.3 ───────────────┴──→ P4 herdr entegrasyonu
                                                + görsel/fiziksel test (ydotool)
```

**Neden bu sıra:**

1. **P0 önce** — lisans olmadan yapılan katkı hukuken belirsiz zeminde kalır.
2. **P1.1 refactor'den önce** — mevcut davranışı dondurmadan god-object'i bölmek körlemesine
   değişiklik olur. Karakterizasyon testleri regresyon alarmıdır.
3. **P2.1 (refactor) P2.2'den (özellik) önce** — çok sayfayı god-object'e eklemek borç üretir.
   Önce yapı, sonra özellik.
4. **P2.2 P3.1'den önce** — okuma katmanını değiştirmeden önce çok-sayfa modeli hazır olmalı;
   yoksa umya geçişi hem kütüphane hem model değişimini aynı anda taşır (iki değişken).
5. **P3.2 P3.3'ten önce** — kirli takibi olmadan xlsx yazma, dokunulmayan veriyi ezer.
6. **P2.5 (mouse) P2.1 VE P2.3'ten sonra** — iki bağımlılığı da var ve ikisi de gerçek:
   `cursor`/`scroll` P2.1'de `Sheet`'e taşınıyor (eski modele göre yazılan hit-testing çöp
   olurdu), ve sayfa sekmesi/göstergesi P2.3'te layout'a giriyor (M8'in tıklayacağı hedef
   ondan önce yok). Hit-testing'i **bir kez, final layout'a göre** yazmak için ikisini de
   bekler. P2.4 ile paraleldir — birbirlerine bağlı değiller.
7. **P4 en son** — entegrasyon, entegre edilecek şey bitmeden doğrulanamaz; fiziksel doğrulama
   hem klavye hem **fare** etkileşimini kapsayacağı için P2.5'i de bekler.

---

## 5. Test Stratejisi

### 5.1 Test katmanları

| Katman | Araç | Ne doğrular |
|---|---|---|
| **U — Birim** | `cargo nextest`, `#[cfg(test)]` | Saf mantık: hücre erişimi, sınırlar, ayrıştırma |
| **K — Karakterizasyon** | aynı | Mevcut davranışın refactor'de değişmediği |
| **R — Round-trip** | Rust test + `openpyxl` doğrulaması | xlsx yazıldıktan sonra veri bütünlüğü |
| **G — Görsel/Fiziksel** | `portal-screenshot` + `ydotool` | Gerçek terminalde gerçek tuş/fare ile davranış |

### 5.2 Her fazın çıkış kapısı (hepsi geçmeden faz kapanmaz)

```
cargo fmt --check                      → biçim
cargo clippy --locked -D warnings      → lint (uyarı = hata)
cargo nextest run --no-fail-fast       → tüm testler, ilk hatada durmadan
grep -rn 'unwrap()' src/ | wc -l       → P1.3 sonrası 0 olmalı
```

### 5.3 Fixture stratejisi

Gerçek dünya karmaşıklığını taşıyan **tek bir üretilmiş fixture** (deterministik, tekrar
üretilebilir): 3 sayfa × (formül + çapraz-sayfa formül + bool + tarih + sayı biçimi + metin).
Üretim script'i repoda tutulur (`tests/fixtures/generate.py` veya Rust'ta build-time).

**Neden üretilmiş, gerçek dosya değil:** müşteri verisi teste giremez (gizlilik), ve
deterministik olmayan fixture flaky test üretir.

---

## 6. Riskler ve Karşı Önlemler

| # | Risk | Olasılık | Etki | Karşı önlem |
|---|---|---|---|---|
| R1 | Yazar PR'ları merge etmez (5 aydır sessiz) | Orta | Orta | Fork kullanılabilir kalır; herdr plugin'i fork'u gösterir. Upstream merge bonus, bağımlılık değil. |
| R2 | LICENSE talebi reddedilir/cevapsız kalır | Düşük-orta | **Yüksek** | Reddedilirse xleak'e (zaten MIT) editör eklemeye dönülür. Karar noktası P0 sonunda. |
| R3 | `.xls` (eski format) desteği kaybı — umya sadece xlsx | Yüksek | Düşük | calamine'i `.xls` için tutmak veya açık hata mesajı. P3.1 test noktası (d). |
| R4 | umya 8.2MB / 86 crate → derleme süresi ve binary şişmesi | Kesin | Düşük-orta | calamine kaldırılarak dengelenir; ölçülüp PR'da raporlanır. |
| R5 | god-object refactor'ü gizli davranış değiştirir | Orta | Yüksek | P1.1 karakterizasyon testleri + refactor'de 63/63'ün değişmeden kalması şartı. |
| R6 | Sayfa gezinme kısayolu mevcut bir tuşla çakışır | Orta | Orta | P2.3'te tuş haritası taraması zorunlu test noktası. |
| R7 | Formül hücresi düzenlenince sessiz veri kaybı | Yüksek | Orta | §3.3 — düzenlemede ham formül gösterilir; kullanıcı ne kaybettiğini görür. |
| R8 | Fare tıkı modal arkasındaki ızgarayı sessizce değiştirir | Orta | **Yüksek** | §3.5 fail-closed yönlendirme + birim testi (modal açıkken tık yok sayılır). |
| R9 | Hit-testing kaydırma/değişken genişlikte yanlış hücreyi seçer | Yüksek | Orta | Saf fonksiyon + tablo-testi (ofset, sınır, başlık bölgeleri) + ydotool ile fiziksel doğrulama. |
| R10 | Terminal fare protokolü farklılıkları (SGR vs X10, Ghostty/kitty) | Düşük | Düşük | crossterm soyutluyor; fiziksel test gerçek Ghostty'de koşar (hedef ortam). |

---

## 7. Başarı Kriterleri

Bu proje şu cümle doğru olduğunda tamamlanmıştır:

> **Kullanıcı, 3 sayfalı formüllü bir xlsx dosyasını herdr'ın dosya yöneticisinde seçip
> düzenleyici olarak açabilir, sayfalar arasında gezebilir (klavye veya FARE ile), bir hücreyi
> fareyle seçip değiştirip kaydedebilir — ve dosyayı Excel'de açtığında dokunmadığı her şey
> (diğer sayfalar, formüller, tarih biçimleri, stiller) yerinde durur.**

Ölçülebilir kapılar:

- [ ] LICENSE (MIT) upstream'de veya fork'ta mevcut
- [ ] `cargo nextest run` ≥ 63 test, **0 fail**
- [ ] `cargo clippy -D warnings` temiz
- [ ] `grep -c 'unwrap()' src/` = **0**
- [ ] 3 sayfalı fixture round-trip: openpyxl doğrulaması **tüm alanlarda eşit**
- [ ] herdr izole örneğinde tab açılıyor, **ydotool ile fiziksel tuş** sayfa değiştiriyor
      (screenshot kanıtı)
- [ ] **ydotool ile fiziksel fare** hücre seçiyor, sürükleyerek aralık seçiyor, tekerlek
      kaydırıyor, sayfa sekmesine tık sayfa değiştiriyor (screenshot kanıtı)
- [ ] Mouse yakalaması artık "çalıp vermiyor" değil: her yakalanan olay bir davranışa karşılık
      geliyor (G6 gerilemesi kapandı)

---

## 8. Git Disiplini

- **Fork:** `CyPack/rustxl` · **upstream:** `only-using-ai/rustxl`
- **Branch:** faz başına kısa ömürlü — `feat/license`, `test/characterization`,
  `refactor/sheet-struct`, `feat/multi-sheet`, `feat/xlsx-write`…
- **Commit:** küçük, atomik, konvansiyonel (`feat:`, `fix:`, `test:`, `refactor:`, `docs:`)
- **Her commit öncesi:** `cargo fmt --check && cargo clippy -D warnings && cargo nextest run`
- **Upstream PR:** faz başına ayrı PR (küçük, gözden geçirilebilir). Planlama dosyaları
  (`.local/`) `.git/info/exclude` ile hariç → PR'lara asla sızmaz.
- **Asla:** yeşil olmayan kapıyla commit, force-push, upstream'e onaysız push.

---

*Oluşturma: 2026-07-25 · Araştırma kanıtları: `cargo nextest` baseline 63/63, umya round-trip*
*probe (`/tmp/umya-probe`), blast-radius grep ölçümleri, herdr `PluginPaneOpen` canlı deneyi.*
