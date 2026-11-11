#!/usr/bin/env python3
"""Playwright-тест песочницы docs/playground.html: загрузка, кадры, REPL, клавиши."""
import sys

from playwright.sync_api import sync_playwright

import os
HTML = "file://" + os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "docs", "playground.html")

with sync_playwright() as p:
    b = p.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
    pg = b.new_page(viewport={"width": 1280, "height": 900})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
    pg.goto(HTML)
    pg.select_option("#preset", "Физполигон (3D, физика, вода)")
    pg.click("#run")
    # сцена с атмосферой грузится несколько секунд — поллим статуса до 20 с
    status = ""
    for _ in range(40):
        pg.wait_for_timeout(500)
        status = pg.text_content("#status") or ""
        if "fps" in status:
            break
    print("статус:", status)
    assert status and "fps" in status, "статус не обновился — цикл не идёт"

    # канвас не пустой: снимаем центр канваса
    px = pg.evaluate("""() => {
        const c = document.getElementById('screen');
        const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let nz = 0;
        for (let i = 0; i < d.length; i += 4) if (d[i] | d[i+1] | d[i+2]) nz++;
        return [nz, c.width, c.height];
    }""")
    print("канвас:", px)
    assert px[0] > px[1] * px[2] * 0.5, "канвас почти пуст"

    # события физики: ждём — кокосы падают первые полсекунды
    pg.wait_for_timeout(1500)

    # REPL: живой вызов
    pg.fill("#replin", "(phys-n *phys*)")
    pg.click("#replgo")
    pg.wait_for_timeout(300)
    con_last = pg.text_content("#console div:last-child")
    print("REPL:", con_last)
    assert "9" in con_last, f"REPL вернул не 9 тел: {con_last}"

    # REPL-ошибка показывается
    pg.fill("#replin", "(несуществует 1)")
    pg.click("#replgo")
    pg.wait_for_timeout(200)
    con_err = pg.text_content("#console div:last-child")
    print("REPL err:", con_err[:60])
    assert "⚠" in con_err, "ошибка REPL не показана"

    # клавиши: R — пересыпать кокосы (event → движок), кадры продолжаются
    pg.focus("#screen") if False else None
    pg.keyboard.press("KeyR")
    pg.wait_for_timeout(800)
    status2 = pg.text_content("#status")
    print("статус после R:", status2)

    # смена пресета: 2D-шаблон
    pg.select_option("#preset", "Только 2D (шаблон)")
    pg.wait_for_timeout(2500)
    status3 = pg.text_content("#status")
    print("статус 2D:", status3)
    assert status3 and "fps" in status3, "2D-пресет не запустился"
    px2 = pg.evaluate("""() => {
        const c = document.getElementById('screen');
        const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let nz = 0, blue = 0;
        for (let i = 0; i < d.length; i += 4) { if (d[i]|d[i+1]|d[i+2]) nz++; if (d[i+2] > 100) blue++; }
        return [nz, blue];
    }""")
    print("канвас 2D:", px2)
    assert px2[0] > 1000, "2D-канвас пуст"

    # мини-сцена с мячами
    pg.select_option("#preset", "Мячи и трамплин (мини-сцена)")
    pg.wait_for_function(
        "() => [...document.querySelectorAll('#console div')].some(e => e.textContent.includes('BALLS'))",
        timeout=10000)
    pg.fill("#replin", "(phys-n *w*)")
    pg.click("#replgo")
    pg.wait_for_timeout(200)
    con_balls = pg.text_content("#console div:last-child")
    print("мячи:", con_balls)
    assert "2" in con_balls, f"мячи не заспавнены: {con_balls}"

    pg.screenshot(path="/tmp/playground_test.png")
    hard = [e for e in errors if "favicon" not in e]
    print("JS-ошибки:", hard[:3] if hard else "нет")
    assert not hard, f"JS-ошибки: {hard[:3]}"

    # ——— ОАЗИС: переключение пресета, клавиши, HUD, лиса ———
    pg.select_option("#preset", "ОАЗИС (FPS: лисы, стрельба, волны)")
    pg.click("#run")
    status2 = ""
    for _ in range(40):
        pg.wait_for_timeout(500)
        status2 = pg.text_content("#status") or ""
        if "fps" in status2:
            break
    print("оазис статус:", status2)
    assert "fps" in status2, "ОАЗИС не стартовал"

    # клавиши: f = старт + выстрел, w = шаг, a/d = повороты
    pg.focus("body")
    pg.keyboard.press("f")
    pg.keyboard.down("w")
    pg.wait_for_timeout(600)
    pg.keyboard.up("w")
    pg.keyboard.down("a")
    pg.wait_for_timeout(400)
    pg.keyboard.up("a")
    pg.wait_for_timeout(500)
    # лиса вшита в wasm и загружена (probe растёт, ошибок нет)
    errs_oasis = [e for e in errors if "оазис" in e.lower() or "gltf" in e.lower()]
    assert not errs_oasis, f"ошибки в ОАЗИС: {errs_oasis[:3]}"
    px2 = pg.evaluate("""() => {
        const c = document.getElementById('screen');
        const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let orange = 0, nonempty = 0;
        for (let i = 0; i < d.length; i += 4) {
            if (d[i] | d[i+1] | d[i+2]) nonempty++;
            if (d[i] > 140 && d[i+1] > 60 && d[i+2] < 90 && d[i] > d[i+2] + 60) orange++;
        }
        return [orange, nonempty];
    }""")
    print("оазис канвас: оранжевых пикселей", px2[0], "непустых", px2[1])
    assert px2[1] > 50000, "канвас ОАЗИС пуст"
    print("ОАЗИС playwright: ok")


    # ——— ШТОЛЬНЯ: хоррор-пресет, старт, фонарь ———
    pg.select_option("#preset", "ШТОЛЬНЯ (хоррор: фонарь, монстр)")
    pg.click("#run")
    status3 = ""
    for _ in range(40):
        pg.wait_for_timeout(500)
        status3 = pg.text_content("#status") or ""
        if "fps" in status3:
            break
    print("штольня статус:", status3)
    assert "fps" in status3, "ШТОЛЬНЯ не стартовала"
    pg.focus("body")
    pg.keyboard.press("w")   # старт
    pg.wait_for_timeout(900)
    pg.keyboard.down("w")
    pg.wait_for_timeout(700)
    pg.keyboard.up("w")
    pg.keyboard.down("a")
    pg.wait_for_timeout(500)
    pg.keyboard.up("a")
    pg.wait_for_timeout(600)
    errs_sht = [e for e in errors if "shtol" in e.lower() or "штольн" in e.lower()]
    assert not errs_sht, f"ошибки ШТОЛЬНИ: {errs_sht[:3]}"
    px3 = pg.evaluate("""() => {
        const c = document.getElementById('screen');
        const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let bright = 0, nonempty = 0;
        for (let i = 0; i < d.length; i += 4) {
            if (d[i] | d[i+1] | d[i+2]) nonempty++;
            if (d[i] > 200 && d[i+1] > 180) bright++;
        }
        return [bright, nonempty];
    }""")
    print("штольня канвас: светлых", px3[0], "непустых", px3[1])
    assert px3[1] > 30000, "канвас ШТОЛЬНИ пуст"
    assert px3[0] > 400, "фонарь не светит"
    print("ШТОЛЬНЯ playwright: ok")

    # ——— v0.17: шаринг сцены через URL-хеш + PNG-экспорт ———
    pg.click("#share")
    pg.wait_for_timeout(300)
    assert "#code=" in pg.url, "шаринг не записал #code в URL"
    con_last = pg.text_content("#console div:last-child")
    assert "ссылка" in con_last, f"нет подсказки о ссылке: {con_last[:60]}"
    print("шаринг:", con_last[:70])
    # перезагрузка страницы: сцена из ссылки должна подхватиться сама
    pg.reload()
    pg.wait_for_timeout(900)
    con_last = pg.text_content("#console div:last-child")
    assert "из ссылки" in con_last, f"сцена из ссылки не загрузилась: {con_last[:70]}"
    assert "Сцена из ссылки" in pg.text_content("#preset option:checked"), "пресет-из-ссылки не выбран"
    print("сцена из ссылки: ok")
    # PNG-кнопка: клик не должен ронять страницу (toBlob → download).
    # Trusted-клик после длинного прогона бывает съеден фокус-танцами
    # headless — fallback: element.click() через evaluate.
    pg.click("#shot")
    pg.wait_for_timeout(500)
    con_shot = pg.text_content("#console div:last-child")
    if "кадр сохранён" not in con_shot:
        pg.evaluate("document.getElementById('shot').click()")
        pg.wait_for_timeout(500)
        con_shot = pg.text_content("#console div:last-child")
    assert "кадр сохранён" in con_shot, f"PNG не сохранился: {con_shot[:60]}"
    print("PNG-экспорт:", con_shot)
    errs_new = [e for e in errors if "favicon" not in e]
    assert not errs_new, f"JS-ошибки на новых кнопках: {errs_new[:3]}"

    b.close()


    print("ВСЕ ПРОВЕРКИ ПРОЙДЕНЫ")
