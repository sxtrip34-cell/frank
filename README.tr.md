<div align="center">

<img src="src-tauri/icons/128x128@2x.png" width="110" alt="Frank">

# Frank

**Ekranının üstünde yaşayan, Claude Code ile çalışan ve seninle konuşan küçük bir ışık topu.**

[English](README.md) · **Türkçe** · [Русский](README.ru.md)

</div>

![Frank'in tüm halleri](docs/character-sheet.png)

## Frank neler yapar

- **Claude Code oturumlarını canlı izler**: Hangi oturumun ne okuduğunu, neyi düzenlediğini ve neyi çalıştırdığını tüm pencerelerinde gösterir.
- **İzinleri ekranın üstünden onaylatır**: Claude Code izin istediğinde Frank **İzin ver / Reddet** gösterir. Terminale dönmen gerekmez.
- **Claude ile sohbet eder**: Kendi Claude aboneliğinle çalışır, **API anahtarı gerekmez**.
- **Seninle konuşur**: **"Frank"** dersin, seni dinler, sesli cevap verir ve dinlemeye devam eder. Konuşurken sözünü kesmek için adını tekrar söylersin.
- **Senin dilini konuşur**: Türkçe, Rusça ve İngilizce anlar, hangi dilde sorduysan o dilde cevap verir. Arayüzü de üç dilde.
- **Projelerini bilir**: Bilgisayarında Claude Code'un çalıştığı her klasörü bilir, hangi uygulamada çalışırsa çalışsın. *"Web sitesi projem ne durumda?"* diye sorduğunda proje klasörünü okur ve anlatır. Dosyalara sadece bakar, hiçbir şeyi değiştirmez.
- **Servislerinle çalışır**: Ayarlar'dan GitHub, Vercel, Stripe, Resend, Notion, Cal.com ya da n8n bağla ve sor: *"Son deploy başarılı mı?"*, *"Depomda neler açık?"*. İşlem de yapabilir (issue açmak, e-posta göndermek, Notion'a yazmak, n8n iş akışı başlatmak), ama sadece ne yapacağını tam gösteren kartta **İzin ver**'e bastıktan sonra. Stripe sadece okunur.
- **Seni hatırlar**: Ona akılda tutulmaya değer bir şey söylersen ya da *"şunu hatırla …"* dersen hafızasına, `C:\Frank\memory` klasörüne yazar ve sonraki her konuşmada bilir.
- **Talimatlarını iletir**: *"Frank, web sitesi projesine söyle başlığı büyütsün"* dediğinde talimatı o projede çalışan Claude Code oturumuna gönderir.
- **Senin için bakar**: Bir ekran görüntüsü yapıştırırsın (**Win + Shift + S**, sonra **Ctrl + V**) ya da üstüne bir dosya bırakırsın, sonra onun hakkında soru sorarsın.

## Ücreti ne

Frank ücretsiz ve açık kaynaklı (MIT lisansı). Kullandıkları:

- **Claude Code girişin** (Claude Pro veya Max): Sohbet için kullanılır ve cevaplar planının kullanım limitinden düşer. API anahtarı yok. Claude hesap ayarlarında *extra usage* kapalı olduğu sürece ek ücret de yok.
- **Kendi bilgisayarında çalışan ücretsiz, açık kaynaklı ses araçları**: Konuşmayı yazıya çevirmek için whisper.cpp, yazıyı sese çevirmek için Piper. Sesin hiçbir zaman bilgisayarından çıkmaz.

## Gereksinimler

- Windows 10 veya 11, 64 bit
- Claude hesabınla giriş yapılmış [Claude Code](https://claude.com/claude-code): `npm install -g @anthropic-ai/claude-code` çalıştır, sonra giriş yapmak için bir kez `claude` çalıştır.
- İsteğe bağlı: NVIDIA ekran kartı, ses tanımayı hem hızlandırır hem doğruluğunu artırır.
- Sadece Frank'i kendin derlemek istersen (hepsi ücretsiz): [Rust](https://rustup.rs), [Node.js 20+](https://nodejs.org) ve **Desktop development with C++** seçeneğiyle [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)

## Kurulum

### İndir (en kolayı)

1. **[Frank-Windows-setup.exe](../../releases/latest/download/Frank-Windows-setup.exe) dosyasını indir.** Dosya [son sürüm](../../releases/latest) sayfasında.
2. **Çalıştır.** Sadece senin kullanıcın için kurulur, yönetici izni gerekmez. Windows kurulum dosyasının kod imzası olmadığını söyleyebilir: *Ek bilgi → Yine de çalıştır*'ı seç.
3. **Kurulumun sonunda ses araçlarına evet de.** Ayrı bir pencerede `%LOCALAPPDATA%\Frank\voice` klasörüne inerler: NVIDIA ekran kartıyla yaklaşık 1,5 GB, kartsız yaklaşık 0,7 GB. Her dosya belirli bir sürüme sabitlidir ve kullanılmadan önce SHA-256 parmak iziyle doğrulanır.
4. **Frank'i başlat:** Başlat menüsünden aç. Ekranının üst ortasında belirir.

İndirdiğin dosyanın gerçek olduğundan emin olmak için PowerShell'de `Get-FileHash Frank-Windows-setup.exe` çalıştır ve çıkan değeri sürüm sayfasındaki SHA-256 ile karşılaştır.

### Kendin derle

1. **Kodu al:** Bu depoyu `git clone` ile indir (ya da ZIP olarak indir) ve klasöründe PowerShell aç.
2. **Derle:**
   ```powershell
   npm install
   npm run sounds
   npm run icons
   $env:CARGO_PROFILE_RELEASE_LTO = "thin"
   $env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16"
   npm run pack
   ```
   Kurulum dosyası `release\` klasörüne çıkar. İki `CARGO_…` satırı, 16-24 GB belleği olan bilgisayarlarda derlemenin belleği tüketmesini önler. İlk derleme birkaç dakika sürer.
3. **Kur:** `release\Frank-Windows-setup.exe` dosyasını çalıştır ve yukarıdaki *İndir* bölümünün 3. adımından devam et.

## İlk adımlar

1. **Claude Code'u bağla:**
   - Bildirim alanındaki Frank simgesine sağ tıkla → **Ayarlar…** → **Claude Code** → **Hook'ları kur…**
   - Claude Code ayarlarında neyin değişeceğini tam olarak görürsün. Hiçbir şey yazılmadan önce tarihli bir yedek alınır.
   - Sonra yeni bir Claude Code oturumu aç.
2. **Eller serbest ses:** Ayarlar → **Genel** → **Uyandırma kelimesi** seçeneğini aç. Kelimeyi de değiştirebilirsin.
3. **Dil:** Ayarlar → **Genel** → **Dil**. "Otomatik" seçeneği Windows'un dilini kullanır.

## Kullanım

| Ne yaparsın | Frank ne yapar |
|---|---|
| Fareyi ekranın en üst ortasına götürürsün | Dışarı bakar |
| Ona tıklarsın | Ada açılır |
| Adayı fareyle sürüklersin | Bıraktığın yerde kalır, hep ekranın içinde durur; bir kenara yaklaşınca o kenar parlar |
| Üst kenarın yakınında bırakırsın | Oraya, bıraktığın hizada yapışır (ortaya bırakırsan kendi yerine oturur) |
| Sol ya da sağ kenarın yakınında bırakırsın | Oraya ince bir sekme olarak yapışır ve kenarın içine saklanır; yerini bir ışık çizgisi gösterir, üstüne gelince geri çıkar |
| Sürüklerken **Esc**'ye basarsın | Ada eski yerine döner |
| **"Frank"** dersin (uyandırma kelimesi açık) | Açılır ve dinler |
| İsteğini **"Frank, …"** diye tek nefeste söylersin | Hemen yapar |
| O konuşurken **"Frank"** dersin | Susar ve dinler |
| Sohbetteki mikrofona tıklarsın | Uyandırma kelimesi olmadan sesli sohbet başlar |
| **Win + Shift + S**, sonra sohbette **Ctrl + V** | Ekran görüntüsünü ekler |
| Adaya bir dosya bırakırsın | Dosyayı yutar, sonra onunla ilgili sorularını cevaplar |
| Claude Code izin ister | Ekranın üstünde **İzin ver / Reddet** belirir |

Sesli sohbet yaklaşık 15 saniye sessizlikten sonra kendiliğinden biter.

## Gizlilik

- Telemetri yok, kendi hesabı da yok.
- Ses **senin bilgisayarında** yazıya çevrilir ve seslendirilir. Claude'a sadece isteğinin metni gider, o da kendi Claude Code'un üzerinden.
- Frank, bilgisayarında Claude Code'un son 60 günde çalıştığı her klasörü okuyabilir ama hiçbirini değiştiremez; birine sen sorduğunda bakar. Bir proje için verdiğin talimatları o projedeki kendi Claude Code oturumun, kendi izin ayarlarıyla yapar.
- Frank'in hafızası `C:\Frank\memory` klasöründe düz metindir: hatırladığı her şey için kısa bir not ve bir dizin (`MEMORY.md`). Yazabildiği tek yer burasıdır. İstediğin zaman okuyabilir, düzenleyebilir ya da silebilirsin; ona *"Benim hakkımda ne hatırlıyorsun?"* diye de sorabilirsin. Şifreleri ve anahtarları asla kaydetmez.
- İsteğe bağlı entegrasyonların anahtarları Windows Kimlik Bilgisi Yöneticisi'nde saklanır, hiçbir zaman diske yazılmaz. Claude onları hiç görmez: çağrıları Frank kendisi yapar, Claude'a sadece cevabı verir.
- Frank'in araçları sadece bu bilgisayarda sunulur (127.0.0.1, her açılışta yenilenen bir şifreyle) ve sohbeti başka hiçbir MCP sunucusu yüklemez, Claude hesabının bağlayıcıları dahil. Frank sadece internete, bağladığın servislere ve bu bilgisayardaki Claude Code oturumlarına ulaşır, başka hiçbir yere değil.

## Sorun giderme

- **Windows kurulum dosyası için uyarı veriyor:** Kurulum dosyasının kod imzası yok (imza her yıl para tutuyor). İndirdiysen önce SHA-256'sını kontrol et, sonra *Ek bilgi → Yine de çalıştır*'ı seç.
- **Derleme "out of memory" hatasıyla duruyor:** Derleme adımındaki iki `CARGO_…` satırını kullan.
- **Frank seni duymuyor:** Windows Ayarları → Gizlilik ve güvenlik → Mikrofon → *Masaüstü uygulamalarının mikrofonunuza erişmesine izin verin* açık olmalı.
- **"Sesli sohbet için %LOCALAPPDATA%\Frank\voice içinde … gerekli" hatası:** Frank kurulumunu tekrar çalıştır ve ses araçlarına evet de. Yarıda kalan indirme kaldığı yerden devam eder.
- **Frank bir projeye ulaşamadığını söylüyor:** Önce o projenin klasöründe Claude Code'u aç. Sonra talimatları ona iletebilir.

## Teşekkür ve lisans

Frank, [MIT lisansıyla](LICENSE) açık kaynaklıdır. Kodu, Louis Raillé'nin [Coucou](https://github.com/Louis-CFM/coucou) projesinden (MIT) yola çıktı. Frank'in karakteri, ikonu ve sesleri ise ona özgüdür ve kodla çizilip sentezlenmiştir. Frank bağımsız bir projedir; Coucou'nun yazarıyla bir bağlantısı yoktur ve onun tarafından onaylanmamıştır. Üçüncü taraf bileşenler ve lisansları [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) dosyasında listelenmiştir.
