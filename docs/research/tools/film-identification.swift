import AppKit
import CoreImage
let width = 7*280, height = 6*210
let sheet = NSImage(size: NSSize(width:width,height:height))
sheet.lockFocus()
NSColor.darkGray.setFill(); NSRect(x:0,y:0,width:width,height:height).fill()
for (index,id) in (4378...4416).enumerated() {
    let name=String(format:"DSC%05d",id)
    let url=URL(fileURLWithPath:".cache/probes/film-identification/\(name).jpg")
    guard let input=CIImage(contentsOf:url), let invert=CIFilter(name:"CIColorInvert") else { continue }
    invert.setValue(input,forKey:kCIInputImageKey)
    guard let output=invert.outputImage,let image=CIContext().createCGImage(output,from:output.extent) else {continue}
    let x=(index%7)*280, y=height-(index/7+1)*210
    NSImage(cgImage:image,size:NSSize(width:260,height:175)).draw(in:NSRect(x:x+10,y:y+27,width:260,height:175))
    (name as NSString).draw(at:NSPoint(x:x+10,y:y+5),withAttributes:[.font:NSFont.systemFont(ofSize:17),.foregroundColor:NSColor.white])
}
sheet.unlockFocus()
let bitmap=NSBitmapImageRep(data:sheet.tiffRepresentation!)!
try bitmap.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:".cache/probes/film-identification.png"))
// Crop the screenshot footer separately, without modifying its source.
let input=NSImage(contentsOfFile:"/var/folders/1y/wnsqq52d1jx36cdrdtvnx1rr0000gn/T/pi-clipboard-c0324b27-dd93-4db6-a50b-f2c267ea4ca3.png")!
let cg=input.cgImage(forProposedRect:nil,context:nil,hints:nil)!
print("Screenshot raster",cg.width,cg.height)
if let cropped=cg.cropping(to:CGRect(x:0,y:cg.height-30,width:1100,height:30)) {
    let rep=NSBitmapImageRep(cgImage:cropped)
    try rep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:".cache/probes/black-user-footer-crop.png"))
}
