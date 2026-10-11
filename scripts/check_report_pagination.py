"""Check actual Chromium PDF pagination using installed Playwright and PyMuPDF."""
import argparse
import subprocess
import tempfile
from pathlib import Path

import fitz


def table(prefix, count):
    rows = "".join(
        f"<tr><th>{prefix}-Row-{i:03}</th><td>{i}.125</td></tr>"
        for i in range(count)
    )
    return (
        f"<table><caption>{prefix} table</caption><thead><tr>"
        f"<th>{prefix} heading</th><th>Exact value</th></tr></thead>"
        f"<tbody>{rows}</tbody></table>"
    )


def check(pages):
    def located(label):
        return [i for i, page in enumerate(pages) if label in page]

    small = [located(f"Small-Row-{i:03}") for i in range(10)]
    assert all(page == small[0] and len(page) == 1 for page in small), "Small table split"
    assert located("Small table") == small[0], "Table caption detached"
    chart = [located(f"Chart {part} marker") for part in ["top", "bottom", "caption"]]
    assert all(page == chart[0] and len(page) == 1 for page in chart), "Chart/caption split"
    rows = [located(f"Data-Row-{i:03}") for i in range(120)]
    assert all(len(page) == 1 for page in rows), "Long table lost or duplicated rows"
    continuation = {page[0] for page in rows}
    assert len(continuation) > 1, "Oversized table did not paginate"
    assert all("Data heading" in pages[i] for i in continuation), "Missing repeated header"
    print("PASS: fitting table and chart stay whole; oversized table retains rows and headers.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", default="node")
    parser.add_argument("--playwright-module", default="playwright")
    args = parser.parse_args()
    css = (Path(__file__).resolve().parent.parent / "src/export/report.css").read_text()

    html = (
        f'<!doctype html><html><head><meta charset="utf-8"><style>{css}</style>'
        '</head><body><div style="height:700px">Page boundary</div>'
        + table("Small", 10)
        + '<div style="height:600px">Chart boundary</div><figure>'
        '<svg viewBox="0 0 720 360" role="img"><title>Chart</title>'
        '<text x="30" y="30">Chart top marker</text>'
        '<rect x="100" y="80" width="200" height="180" fill="#219EBC"/>'
        '<text x="30" y="330">Chart bottom marker</text></svg>'
        '<figcaption>Chart caption marker</figcaption></figure>'
        '<details open><summary>Exact chart data</summary>'
        + table("Data", 120)
        + '</details></body></html>'
    )
    render = """
    const { chromium } = require(process.argv[1]);
    (async () => {
      const browser = await chromium.launch({headless:true});
      try {
        const page = await browser.newPage();
        await page.goto(process.argv[2]);
        await page.pdf({path:process.argv[3],format:'A4',printBackground:true});
      } finally { await browser.close(); }
    })().catch(e => { console.error(e); process.exit(1); });
    """
    with tempfile.TemporaryDirectory(prefix="tessera-pagination-") as directory:
        source, pdf = Path(directory) / "report.html", Path(directory) / "report.pdf"
        source.write_text(html, encoding="utf-8")
        subprocess.run(
            [args.node, "-e", render, args.playwright_module, source.as_uri(), str(pdf)],
            check=True,
        )
        with fitz.open(pdf) as document:
            pages = [page.get_text() for page in document]

    check(pages)


if __name__ == "__main__":
    main()
