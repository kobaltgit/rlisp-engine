# rlisp-engine v0.20.0 — честная альфа, сохранения прогресса

## Что нового с v0.19.0

- Спрайты без чёрных подложек: per-texel альфа (alpha-test в opaque,
  блендинг в прозрачном проходе) — демоны/пикапы РУИНЕРА выглядят как
  в классических шутерах.
- `save-table`/`load-table`: JSON-сохранения игр без зависимостей;
  РУИНЕР хранит рекорд очков и уровень в `saves/ruiner.json`.
- Звук: шаги при движении, рыки демонов в погоне.

## Быстрый старт

```bash
tar -xzf rlisp-engine-v0.20.0-linux-x64.tar.gz && cd rlisp-engine-0.20.0
./rlisp-engine --script lisp/main_ruiner.lisp
```

Экспорт игры одним файлом:
```bash
python3 scripts/export_game.py --game assets/lisp/main_ruiner.lisp \
  --title "РУИНЕР" --out ruiner.html
```

## Лицензии

Код — MIT. Ассеты игр процедурные.
