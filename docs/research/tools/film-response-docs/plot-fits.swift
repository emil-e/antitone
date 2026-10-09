import AppKit
import Foundation
let root=".cache/probes/film-response-docs/"
let lines=try String(contentsOfFile:root+"fit-predictions.csv",encoding:.utf8).components(separatedBy:.newlines).filter { !$0.isEmpty }
let rows=lines.dropFirst().map { $0.split(separator:",").map(String.init) }
let size=NSSize(width:1080,height:620)
let image=NSImage(size:size)
image.lockFocus();NSColor.white.setFill();NSRect(origin:.zero,size:size).fill()
func text(_ value:String,_ x:Double,_ y:Double,_ font:Double=11,_ color:NSColor = .black) {
    (value as NSString).draw(at:NSPoint(x:x,y:y),withAttributes:[.font:NSFont.systemFont(ofSize:font),.foregroundColor:color])
}
text("Published film curves and smooth model fits",18,589,20)
let keys=["density","current_width","toe_only","toe_shoulder"]
let colors:[NSColor]=[.black,NSColor.systemOrange,NSColor.systemTeal,NSColor.systemPurple]
for i in 0..<4 {
    let x=18+Double(i)*263
    colors[i].setStroke();let p=NSBezierPath();p.move(to:NSPoint(x:x,y:574));p.line(to:NSPoint(x:x+24,y:574));p.lineWidth=2;p.stroke()
    text(["Published drawing","Current assumed toe width","Fitted toe only","Fitted toe + shoulder"][i],x+30,567)
}
for (si,stock) in ["portra400","ektar100"].enumerated() {
    for (ci,channel) in ["B","G","R"].enumerated() {
        let set=rows.filter {$0[0]==stock && $0[1]==channel}
        let ox=Double(ci*360),oy=Double((1-si)*260+25)
        let left=ox+46,bottom=oy+32,width=294.0,height=190.0
        let lo=Double(set.first![2])!,hi=Double(set.last![2])!
        func xy(_ x:Double,_ y:Double)->NSPoint {NSPoint(x:left+(x-lo)/(hi-lo)*width,y:bottom+y/3.3*height)}
        text((stock=="portra400" ? "Portra 400" : "Ektar 100")+" / "+channel,ox+46,oy+232,14)
        NSColor(calibratedWhite:0.87,alpha:1).setStroke()
        for d in 0...3 {
            let p=NSBezierPath();p.move(to:xy(lo,Double(d)));p.line(to:xy(hi,Double(d)));p.lineWidth=0.5;p.stroke()
            text(String(d),left-16,bottom+Double(d)/3.3*height-6)
        }
        for step in 0...4 {
            let x=lo+(hi-lo)*Double(step)/4
            text(String(format:"%.1f",x),left+width*Double(step)/4-12,bottom-17,10)
        }
        NSColor.black.setStroke();let border=NSBezierPath(rect:NSRect(x:left,y:bottom,width:width,height:height));border.lineWidth=0.7;border.stroke()
        for (column,color) in zip(3...6,colors) {
            color.setStroke();let p=NSBezierPath()
            for (j,row) in set.enumerated() {
                let pt=xy(Double(row[2])!,Double(row[column])!)
                if j==0 {p.move(to:pt)} else {p.line(to:pt)}
            }
            p.lineWidth=column==3 ? 2.4 : 1.2
            if column==4 {p.setLineDash([4,3],count:2,phase:0)}
            p.stroke()
        }
    }
}
text("Vertical: measured density. Horizontal: log exposure. Curves are extracted publisher traces, not raw measurement tables.",18,5,11)
image.unlockFocus()
let bitmap=NSBitmapImageRep(data:image.tiffRepresentation!)!
try bitmap.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:root+"curve-fits.png"))
