// Builds the fixture packages from tests/reference-docx-styles.test.ts and
// tests/reference-docx-patch.test.ts, and runs the built TypeScript package
// as the behavior oracle. stdout is unused. The first argument is the output
// path. A thrown conversion writes `code=` lines instead of a docx.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import JSZip from "jszip";
import { Document, Packer, Paragraph } from "docx";
import {
  convertMarkdownToBuffer,
  convertMarkdownWithReferenceDocxToBuffer,
  patchMarkdownInDocxToBuffer,
} from "@mohtasham/md-to-docx";

const here = path.dirname(fileURLToPath(import.meta.url));
const fixturePath = path.join(here, "fixtures", "reference-template.docx");

const THEME_XML = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Corporate Theme">
  <a:themeElements><a:clrScheme name="Corporate"><a:dk1><a:srgbClr val="112233"/></a:dk1></a:clrScheme>
  <a:fontScheme name="Corporate Fonts"><a:majorFont><a:latin typeface="Aptos Display"/></a:majorFont><a:minorFont><a:latin typeface="Aptos"/></a:minorFont></a:fontScheme>
  <a:fmtScheme name="Corporate Format"/></a:themeElements>
</a:theme>`;

const STYLES_XML = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi"/></w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="CorpBody"><w:name w:val="Normal"/><w:rPr><w:color w:val="334455"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="CorpTitle"><w:name w:val="Corporate Title"/><w:basedOn w:val="CorpBody"/></w:style>
  <w:style w:type="paragraph" w:styleId="CorpH1"><w:name w:val="Executive Heading"/><w:basedOn w:val="CorpBody"/><w:rPr><w:color w:val="AA2200"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="CorpH2"><w:name w:val="Division Heading"/><w:basedOn w:val="CorpBody"/></w:style>
  <w:style w:type="paragraph" w:styleId="CorpQuote"><w:name w:val="Quote"/><w:basedOn w:val="CorpBody"/></w:style>
  <w:style w:type="paragraph" w:styleId="CorpCode"><w:name w:val="Code Block"/><w:basedOn w:val="CorpBody"/></w:style>
  <w:style w:type="paragraph" w:styleId="CorpCaption"><w:name w:val="Caption"/><w:basedOn w:val="CorpBody"/></w:style>
  <w:style w:type="paragraph" w:styleId="CorpList"><w:name w:val="List Paragraph"/><w:basedOn w:val="CorpBody"/><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style>
  <w:style w:type="table" w:styleId="CorpTable"><w:name w:val="Normal Table"/></w:style>
  <w:style w:type="character" w:styleId="CorpStrong"><w:name w:val="Strong"/><w:rPr><w:b/></w:rPr></w:style>
  <w:style w:type="character" w:styleId="CorpEmphasis"><w:name w:val="Emphasis"/><w:rPr><w:i/></w:rPr></w:style>
  <w:style w:type="character" w:styleId="CorpInline"><w:name w:val="Inline Code"/><w:rPr><w:rFonts w:ascii="Consolas"/></w:rPr></w:style>
  <w:style w:type="character" w:styleId="CorpLink"><w:name w:val="Hyperlink"/><w:rPr><w:color w:val="008844"/></w:rPr></w:style>
</w:styles>`;

function addRelationship(xml, id, type, target) {
  const relationship = `<Relationship Id="${id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/${type}" Target="${target}"/>`;
  if (xml.includes("</Relationships>")) {
    return xml.replace("</Relationships>", `${relationship}</Relationships>`);
  }
  return xml.replace(
    /<Relationships\b([^>]*)\/>/,
    `<Relationships$1>${relationship}</Relationships>`,
  );
}

async function createReferenceFixture(mutate) {
  const zip = await JSZip.loadAsync(readFileSync(fixturePath));
  zip.file("word/styles.xml", STYLES_XML);
  zip.file("word/theme/theme1.xml", THEME_XML);
  zip.file(
    "word/fontTable.xml",
    `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:fonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:font w:name="Aptos"><w:family w:val="swiss"/></w:font></w:fonts>`,
  );

  const relsPath = "word/_rels/document.xml.rels";
  let rels = await zip.file(relsPath).async("string");
  if (!rels.includes('/theme"')) {
    rels = addRelationship(rels, "rIdTheme", "theme", "theme/theme1.xml");
  }
  zip.file(relsPath, rels);

  const typesPath = "[Content_Types].xml";
  let types = await zip.file(typesPath).async("string");
  if (!types.includes("/word/theme/theme1.xml")) {
    types = types.replace(
      "</Types>",
      `<Override PartName="/word/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>`,
    );
  }
  zip.file(typesPath, types);
  if (mutate) await mutate(zip);
  return zip.generateAsync({ type: "nodebuffer", compression: "DEFLATE" });
}

async function mutateCorporate(name) {
  if (!name) return undefined;
  if (name === "duplicate-styles") {
    return async (zip) => {
      const styles = await zip.file("word/styles.xml").async("string");
      zip.file(
        "word/styles.xml",
        styles.replace(
          "</w:styles>",
          '<w:style w:type="paragraph" w:styleId="CorpH1Duplicate"><w:name w:val="Executive Heading"/></w:style><w:style w:type="paragraph" w:styleId="CorpQuoteDuplicate"><w:name w:val="Quote"/></w:style></w:styles>',
        ),
      );
    };
  }
  if (name === "malformed-styles") {
    return (zip) => {
      zip.file("word/styles.xml", STYLES_XML.replace("</w:styles>", "<w:style></w:styles>"));
    };
  }
  if (name === "malformed-header") {
    return (zip) => {
      zip.file(
        "word/header1.xml",
        '<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p></w:hdr>',
      );
    };
  }
  if (name === "chart-rel") {
    return async (zip) => {
      const relsPath = "word/_rels/header1.xml.rels";
      const rels = await zip.file(relsPath).async("string");
      zip.file(relsPath, addRelationship(rels, "rIdChart", "chart", "charts/chart1.xml"));
    };
  }
  throw new Error(`unknown corporate mutation ${name}`);
}

function writeError(out, error) {
  const context = error && typeof error === "object" && error.context ? error.context : {};
  const limit = context.limit === undefined || context.limit === null ? "" : String(context.limit);
  const lines = [
    `code=${context.code ?? ""}`,
    `phase=${context.phase ?? ""}`,
    `role=${context.role ?? ""}`,
    `entry=${context.entry ?? ""}`,
    `limit=${limit}`,
    `relationshipType=${context.relationshipType ?? ""}`,
    `message=${String(error && error.message ? error.message : error).replaceAll("\n", " ")}`,
  ];
  writeFileSync(out, lines.join("\n"));
}

function readRequest(file) {
  const request = JSON.parse(readFileSync(file, "utf8"));
  if (request.options && request.options.signal === "aborted") {
    const controller = new AbortController();
    controller.abort();
    request.options.signal = controller.signal;
  }
  return request;
}

async function placeholders(names) {
  return Packer.toBuffer(
    new Document({
      sections: [
        {
          children: names.map((placeholder) => new Paragraph(`{{${placeholder}}}`)),
        },
      ],
    }),
  );
}

async function rawPackage(kind) {
  if (kind === "not-a-zip") return Buffer.from("not a zip");
  if (kind === "hello") {
    const zip = new JSZip();
    zip.file("hello.txt", "hello");
    return zip.generateAsync({ type: "nodebuffer" });
  }
  if (kind === "unsafe") {
    const zip = new JSZip();
    zip.file("../word/document.xml", "unsafe");
    return zip.generateAsync({ type: "nodebuffer" });
  }
  if (kind === "invalid-package") {
    const zip = new JSZip();
    zip.file(
      "[Content_Types].xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"></Types>`,
    );
    zip.file(
      "_rels/.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>`,
    );
    zip.file(
      "word/document.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p/></w:body></w:document>`,
    );
    zip.file(
      "word/_rels/document.xml.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>`,
    );
    zip.file(
      "word/styles.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"></w:styles>`,
    );
    return zip.generateAsync({ type: "nodebuffer" });
  }
  throw new Error(`unknown raw package ${kind}`);
}

async function companySource() {
  const source = await convertMarkdownToBuffer("Content");
  const zip = await JSZip.loadAsync(source);
  const app = await zip.file("docProps/app.xml").async("string");
  zip.file(
    "docProps/app.xml",
    app.replace(/\/>\n?$/u, "><Company>Existing Co</Company></Properties>"),
  );
  return zip.generateAsync({ type: "nodebuffer" });
}

const [out, command, ...rest] = process.argv.slice(2);

try {
  let bytes;
  if (command === "template") {
    bytes = readFileSync(fixturePath);
  } else if (command === "corporate") {
    bytes = await createReferenceFixture(await mutateCorporate(rest[0]));
  } else if (command === "placeholders") {
    bytes = await placeholders(rest);
  } else if (command === "raw") {
    bytes = await rawPackage(rest[0]);
  } else if (command === "company-source") {
    bytes = await companySource();
  } else if (command === "ts-convert") {
    const request = readRequest(rest[0]);
    bytes = await convertMarkdownToBuffer(request.markdown, request.options ?? {});
  } else if (command === "ts-reference") {
    const request = readRequest(rest[1]);
    bytes = await convertMarkdownWithReferenceDocxToBuffer(
      request.markdown,
      readFileSync(rest[0]),
      request.options ?? {},
    );
  } else if (command === "ts-patch") {
    const request = readRequest(rest[1]);
    bytes = await patchMarkdownInDocxToBuffer(
      readFileSync(rest[0]),
      request.patches,
      request.options ?? {},
    );
  } else if (command === "ts-error-reference") {
    const request = readRequest(rest[1]);
    try {
      await convertMarkdownWithReferenceDocxToBuffer(
        request.markdown,
        readFileSync(rest[0]),
        request.options ?? {},
      );
      writeError(out, new Error("TypeScript reference conversion succeeded"));
    } catch (error) {
      writeError(out, error);
    }
    process.exit(0);
  } else if (command === "ts-error-patch") {
    const request = readRequest(rest[1]);
    try {
      await patchMarkdownInDocxToBuffer(
        readFileSync(rest[0]),
        request.patches,
        request.options ?? {},
      );
      writeError(out, new Error("TypeScript patch succeeded"));
    } catch (error) {
      writeError(out, error);
    }
    process.exit(0);
  } else {
    throw new Error(`unknown oracle command ${command}`);
  }
  writeFileSync(out, bytes);
} catch (error) {
  console.error(error && error.stack ? error.stack : error);
  process.exit(1);
}
