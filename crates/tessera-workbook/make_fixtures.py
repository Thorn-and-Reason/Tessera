"""Create disposable parser adversaries with Python's standard ZIP implementation."""
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED

root = Path(__file__).parent / "fixtures"
root.mkdir(exist_ok=True)
common = {
    "_rels/.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
    "xl/_rels/workbook.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>',
    "xl/workbook.xml": '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>',
    "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>',
}
def fixture(name, sheet, strings=None, epoch=False, styles=None):
    with ZipFile(root / name, "w", compression=ZIP_DEFLATED) as archive:
        for path, text in common.items():
            if path == "xl/workbook.xml" and epoch:
                text = text.replace("<sheets>", '<workbookPr date1904="1"/><sheets>')
            archive.writestr(path, text)
        archive.writestr("xl/worksheets/sheet1.xml", '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">' + sheet + '</worksheet>')
        if strings is not None:
            archive.writestr("xl/sharedStrings.xml", strings)
        if styles is not None:
            archive.writestr("xl/styles.xml", styles)

fixture("tiny.xlsx", '<dimension ref="A1:B2"/><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="inlineStr"><is><t>Value</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>00123</t></is></c><c r="B2"><f>1+2</f><v>3</v></c></row></sheetData>', '<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" uniqueCount="1"><si><t>ID</t></si></sst>')
fixture("unique-count-bomb.xlsx", '<sheetData/>', '<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" uniqueCount="1000000000000"><si><t>x</t></si></sst>')
fixture("sparse-distant.xlsx", '<dimension ref="A1:XFD1048576"/><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="1048576"><c r="XFD1048576"><v>2</v></c></row></sheetData>')
fixture("shared-formula-index.xlsx", '<dimension ref="A1"/><sheetData><row r="1"><c r="A1"><f t="shared" si="1000000000000" ref="A1">1+2</f><v>3</v></c></row></sheetData>')
fixture("zip-expansion.xlsx", '<sheetData/>', '<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" uniqueCount="1"><si><t>' + 'x' * (16 * 1024 * 1024) + '</t></si></sst>')
styles = '<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cellXfs count="2"><xf numFmtId="14"/><xf numFmtId="46"/></cellXfs></styleSheet>'
dates = '<dimension ref="A1:B2"/><sheetData><row r="1"><c r="A1" s="0"><v>60</v></c><c r="B1" s="1"><v>1.5</v></c></row><row r="2"><c r="A2"><f>1+2</f></c><c r="B2"><v>9007199254740993</v></c></row></sheetData>'
fixture("dates-1900.xlsx", dates, styles=styles)
fixture("dates-1904.xlsx", dates, epoch=True, styles=styles)

def multiple_sheets(name, names):
    namespace = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
    rel_ns = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
    with ZipFile(root / name, "w", compression=ZIP_DEFLATED) as archive:
        for path, text in common.items():
            if path not in ("xl/workbook.xml", "xl/_rels/workbook.xml.rels"):
                archive.writestr(path, text)
        sheets = ''.join(f'<sheet name="{sheet}" sheetId="{i+1}" r:id="rId{i+1}"/>' for i, sheet in enumerate(names))
        archive.writestr("xl/workbook.xml", f'<workbook xmlns="{namespace}" xmlns:r="{rel_ns}"><sheets>{sheets}</sheets></workbook>')
        relations = ''.join(f'<Relationship Id="rId{i+1}" Type="{rel_ns}/worksheet" Target="worksheets/sheet{i+1}.xml"/>' for i in range(len(names)))
        archive.writestr("xl/_rels/workbook.xml.rels", '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + relations + '</Relationships>')
        for i in range(len(names)):
            value = 1 if i == 0 else 777
            archive.writestr(f"xl/worksheets/sheet{i+1}.xml", f'<worksheet xmlns="{namespace}"><dimension ref="A1"/><sheetData><row r="1"><c r="A1"><v>{value}</v></c></row></sheetData></worksheet>')

multiple_sheets("multi-sheet.xlsx", ["Data one", "Ail ddalen"])
multiple_sheets("too-many-sheets.xlsx", [f"Sheet {i}" for i in range(257)])
multiple_sheets("oversized-sheet-name.xlsx", ["x" * 1025])
