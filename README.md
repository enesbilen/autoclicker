# AutoClicker

macOS (Apple Silicon) ve Windows (x64) için hızlı, hafif otomatik tıklayıcı. Rust ile yazıldı.

## Özellikler

- **Aralık:** saat / dakika / saniye / salise (ms), en düşük 1 ms
- **Fare:** sol, sağ, orta düğme, çift tık, imleç konumu veya sabit koordinat
- **Klavye:** harf, rakam, F1–F12, Enter, Space, ok tuşları; Ctrl / Shift / Alt(Opt) / Win(Cmd) kombinasyonları
- **Basılı tutma:** fare düğmesi veya tuşu belirlediğin ms kadar basılı tutar
- **Tıklama dizisi:** birden çok konuma sırayla tıklama, her adım için düğme ve bekleme süresi
- **Bitiş:** durdurulana kadar, belirli sayıda (dizide döngü sayısı) veya süre sınırı (ör. 5 dk)
- **Bildirim:** iş bitince ses (macOS'ta ayrıca bildirim)
- **Kısayollar:** başlat ve durdur için tuş ataması (varsayılan F6; aynı tuş = aç/kapa)
- **Menü çubuğu / sistem tepsisi:** Başlat/Durdur, pencereyi göster, çık
- **Acil durdurma:** fare modunda imleci ekranın sol üst köşesine götür
- **Rapor:** toplam tıklama, süre, ortalama tık/sn
- **Güncelleme kontrolü:** yeni sürüm varsa uygulama içinde haber verir, **Güncelle** ile indirip açar

## Kurulum

[Releases](../../releases) sayfasından son sürümü indir.

**macOS:** `AutoClicker-macos-arm64.dmg` dosyasını aç, `AutoClicker.app`'i Applications'a sürükle.
Uygulama henüz imzasız olduğu için ilk açılışta engellenirse uygulamaya sağ tıklayıp **Aç** de, ya da:

```sh
xattr -dr com.apple.quarantine /Applications/AutoClicker.app
```

**Windows:** `AutoClicker-windows-x64.exe` dosyasını çalıştır. SmartScreen uyarısı çıkarsa
**Ek bilgi → Yine de çalıştır** de.

## macOS izinleri

**Sistem Ayarları → Gizlilik ve Güvenlik** altında uygulamaya **Erişilebilirlik** ve **Girdi İzleme**
izni ver. Bu izinler olmadan tıklama ve kısayollar çalışmaz. İzni verdikten sonra uygulamayı yeniden başlat.

## Kaynaktan derleme

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo run --release
```

## Release oluşturma

`Cargo.toml` içindeki `version` değerini güncelle, sonra etiket at:

```sh
git tag v0.4.0
git push origin v0.4.0
```

GitHub Actions macOS (Apple Silicon, `.dmg`) ve Windows (`.exe`) sürümlerini derleyip Releases'e ekler.
