#include <libraw/libraw.h>
#include <cstdio>
#include <fstream>
#include <string>
int main() {
    for(int id=4378;id<=4416;id++) {
        char name[96], out[128];
        std::snprintf(name,sizeof(name),"samples/DSC%05d.ARW",id);
        std::snprintf(out,sizeof(out),".cache/probes/film-identification/DSC%05d.jpg",id);
        LibRaw raw;
        int err=raw.open_file(name); if(!err)err=raw.unpack_thumb();
        if(err) { std::fprintf(stderr,"%s: %s\n",name,libraw_strerror(err));continue; }
        auto *image=raw.dcraw_make_mem_thumb(&err);
        if(!image||err||image->type!=LIBRAW_IMAGE_JPEG) { std::fprintf(stderr,"%s: no JPEG thumbnail\n",name);if(image)LibRaw::dcraw_clear_mem(image);continue; }
        std::ofstream file(out,std::ios::binary);file.write(reinterpret_cast<const char *>(image->data),image->data_size);
        LibRaw::dcraw_clear_mem(image);
    }
}
