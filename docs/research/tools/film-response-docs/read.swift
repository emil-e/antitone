import Foundation
import PDFKit
import AppKit
for name in CommandLine.arguments.dropFirst() {
    let url=URL(fileURLWithPath:".cache/probes/film-response-docs/"+name)
    guard let doc=PDFDocument(url:url) else {fatalError("not PDF: \(name)")}
    var text=""
    for i in 0..<doc.pageCount {
        let page=doc.page(at:i)!
        let s=page.string ?? ""
        text += "\nPAGE \(i+1)\n"+s+"\n"
        if s.lowercased().contains("characteristic curves") {
            let image=page.thumbnail(of:NSSize(width:1100,height:1500),for:.mediaBox)
            let rep=NSBitmapImageRep(data:image.tiffRepresentation!)!
            try rep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:url.path+"-page\(i+1).png"))
        }
    }
    try text.write(toFile:url.path+".txt",atomically:true,encoding:.utf8)
}
