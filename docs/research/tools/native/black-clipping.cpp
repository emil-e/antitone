#include "../../../../native/rendering_transform.h"
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <vector>
namespace OCIO = OCIO_NAMESPACE;
using RGB = std::array<float, 3>;
OCIO::ConstCPUProcessorRcPtr raw_render(unsigned target) {
    auto group=OCIO::GroupTransform::Create();
    auto basis=OCIO::BuiltinTransform::Create();
    basis->setStyle("ACEScg_to_ACES2065-1");
    group->appendTransform(basis);
    const auto &p=antitone::output_parameters(target);
    group->appendTransform(OCIO::FixedFunctionTransform::Create(OCIO::FIXED_FUNCTION_ACES_OUTPUT_TRANSFORM_20,p.data(),p.size()));
    return OCIO::Config::CreateRaw()->getProcessor(group)->getDefaultCPUProcessor();
}
RGB apply(OCIO::ConstCPUProcessorRcPtr cpu, RGB p) {cpu->applyRGB(p.data());return p;}
float encode(float x) {return x <= .0031308f ? 12.92f*x : 1.055f*std::pow(x,1.f/2.4f)-.055f;}
float maximum(RGB p) {return *std::max_element(p.begin(),p.end());}
float minimum(RGB p) {return *std::min_element(p.begin(),p.end());}
std::vector<RGB> grid(const char *path) {
    std::ifstream input(path,std::ios::binary);if(!input)throw std::runtime_error(path);
    std::vector<RGB> result;std::array<float,4> value;
    while(input.read(reinterpret_cast<char *>(value.data()),sizeof(value)))result.push_back({value[0],value[1],value[2]});
    return result;
}
int main() try {
    std::printf("OCIO %s / sRGB + Display P3 100nit / no production changes\n",OCIO::GetVersion());
    for(unsigned target=0;target<2;target++) {
        auto cpu=antitone::rendering(target)->getDefaultCPUProcessor();
        auto raw=raw_render(target);
        std::printf("\nTARGET %u: neutral AP1 ramp: input -> raw ACES RGB -> limited RGB -> encoded/code16\n",target);
        for(float x:{0.f,1e-12f,1e-10f,1e-8f,1e-7f,1e-6f,1e-5f,1e-4f,1e-3f,.003f,.01f,.03f,.1f,.18f}) {
            RGB p{x,x,x};auto out=apply(cpu,p), unlimited=apply(raw,p);
            std::printf("AP1=%.9g raw=(%.9g %.9g %.9g) limited=(%.9g %.9g %.9g) encoded=%.9g code16=%.0f\n",x,unlimited[0],unlimited[1],unlimited[2],out[0],out[1],out[2],encode(out[0]),std::round(65535.f*encode(out[0])));
        }
        std::printf("Near-black chromatic AP1 rays: level, ray, raw/limited maxima, zero channel count\n");
        for(float x:{1e-5f,1e-4f,1e-3f,.01f,.1f}) for(RGB ray:{RGB{1,0,0},RGB{0,1,0},RGB{0,0,1},RGB{1,.01f,.01f},RGB{1,1,.5f}}) {
            RGB p{ray[0]*x,ray[1]*x,ray[2]*x};auto out=apply(cpu,p),unlimited=apply(raw,p);
            unsigned zero=0;for(float c:out)zero+=c==0;
            std::printf("level=%.5g ray=(%.2g %.2g %.2g) raw=(%.9g %.9g %.9g) limited_max=%.9g zero_channels=%u\n",x,ray[0],ray[1],ray[2],unlimited[0],unlimited[1],unlimited[2],maximum(out),zero);
        }
        auto working=grid(".cache/probes/camera-working-ap1-model2.f32");
        auto scans=grid(".cache/probes/camera-scan-ap0-model2.f32");
        unsigned negative=0,allNonpositive=0,zero=0,rawZero=0,clipNewZero=0,negativeRaw=0,dim=0;
        for(RGB p:working) {
            negative+=minimum(p)<0;allNonpositive+=maximum(p)<=0;
            auto out=apply(cpu,p),unlimited=apply(raw,p);
            zero+=maximum(out)==0;rawZero+=maximum(unlimited)==0;clipNewZero+=maximum(out)==0&&maximum(unlimited)>0;
            negativeRaw+=minimum(unlimited)<0;dim+=encode(maximum(out))<1.f/255.f;
        }
        std::printf("DIAGNOSTIC (NOT USER GRADE) grids DSC04378,DSC04397,DSC04416 1024 each. roll gamma .65/balance(.2,-.1)/exposure .5; photo WB(-.25,.15)/exposure .25. pixels=%zu negative_AP1=%u all_nonpositive_AP1=%u limited_black=%u unlimited_black=%u limited_black_with_positive_unlimited=%u negative_unlimited_output=%u max_encoded_below_1of255=%u\n",working.size(),negative,allNonpositive,zero,rawZero,clipNewZero,negativeRaw,dim);
        for(unsigned start=0;start<working.size();start+=1024) {
            unsigned black=0,neg=0,allneg=0,subcode=0;
            for(unsigned i=start;i<std::min(start+1024u,unsigned(working.size()));i++){
                RGB p=working[i],out=apply(cpu,p);black+=maximum(out)==0;neg+=minimum(p)<0;allneg+=maximum(p)<=0;subcode+=encode(maximum(out))<1.f/255.f;
                if(maximum(out)==0) {
                    RGB s=scans[i];unsigned index=i-start;
                    std::printf("black_pixel grid=%u grid_xy=(%u,%u) source_xy=(%u,%u) scan_AP0=(%.9g %.9g %.9g) working_AP1=(%.9g %.9g %.9g)\n",start/1024,index%32,index/32,12+(2*(index%32)+1)*8640/64,12+(2*(index/32)+1)*5760/64,s[0],s[1],s[2],p[0],p[1],p[2]);
                }
            }
            std::printf("grid=%u negative_AP1=%u all_nonpositive_AP1=%u black=%u max_encoded_below_1of255=%u\n",start/1024,neg,allneg,black,subcode);
        }
    }
    return 0;
} catch(const std::exception &e) {std::fprintf(stderr,"%s\n",e.what());return 1;}
