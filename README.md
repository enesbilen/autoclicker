# AutoClicker

macOS ve Windows için hızlı, hafif otomatik tıklayıcı. Rust ile yazıldı, tek çalıştırılabilir dosya.

## Özellikler

- Saat / dakika / saniye / salise (ms) cinsinden aralık (en düşük 1 ms)
- Fare otomasyonu: sol, sağ, orta düğme, çift tık, imleç konumu veya sabit koordinat
- Klavye otomasyonu: harf, rakam, F1–F12, Enter, Space, ok tuşları
- Tekrar sayısı veya durdurulana kadar
- Atanabilir başlat / durdur kısayolları (varsayılan F6, aynı tuş = aç/kapa)
- Tıklama dizisi: birden çok konuma sırayla tıklama, adım başına bekleme
- Basılı tutma, tuş kombinasyonları, süre sınırı, bitişte ses/bildirim
- Menü çubuğu / sistem tepsisi: Başlat/Durdur, pencereyi göster, çık
- Acil durdurma: imleci sol üst köşeye götür
- Tıklama raporu: toplam tıklama, süre, ortalama tık/sn

## İndirme

[Releases](../../releases) sayfasından işletim sistemine uygun dosyayı indir.

## macOS izinleri

İlk çalıştırmada **Sistem Ayarları → Gizlilik ve Güvenlik** altında uygulamaya (veya Terminal'e)
**Erişilebilirlik** ve **Girdi İzleme** izni ver. Bu izinler olmadan tıklama ve kısayollar çalışmaz.
İlk açılışta engellenirse uygulamaya sağ tıklayıp **Aç** de veya `xattr -dr com.apple.quarantine /Applications/AutoClicker.app` çalıştır.

## Kaynaktan derleme

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo run --release
```

## Release oluşturma

```sh
git tag v0.1.0
git push origin v0.1.0
```

GitHub Actions macOS (Apple Silicon) ile Windows sürümlerini derleyip Releases'e ekler.
