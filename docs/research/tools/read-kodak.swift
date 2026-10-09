import Foundation
import PDFKit
let doc=PDFDocument(url:URL(fileURLWithPath:".cache/probes/kodak-h740.pdf"))!
var text=""
for i in 0..<doc.pageCount {text += "\nPAGE \(i+1)\n" + (doc.page(at:i)?.string ?? "") + "\n"}
try text.write(toFile:".cache/probes/kodak-h740.txt",atomically:true,encoding:.utf8)
