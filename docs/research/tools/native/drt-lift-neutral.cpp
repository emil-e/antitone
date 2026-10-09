#include "../../../../native/rendering_transform.h"
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <vector>
namespace OCIO=OCIO_NAMESPACE;
using RGB=std::array<float,3>;
int main(int argc,char **argv) try {
    auto forward=antitone::rendering(0)->getDefaultCPUProcessor();
    auto group=OCIO::GroupTransform::Create();
    const auto &p=antitone::output_parameters(0);
    auto inverse=OCIO::FixedFunctionTransform::Create(OCIO::FIXED_FUNCTION_ACES_OUTPUT_TRANSFORM_20,p.data(),p.size());
    inverse->setDirection(OCIO::TRANSFORM_DIR_INVERSE);group->appendTransform(inverse);
    auto basis=OCIO::BuiltinTransform::Create();basis->setStyle("ACEScg_to_ACES2065-1");basis->setDirection(OCIO::TRANSFORM_DIR_INVERSE);group->appendTransform(basis);
    auto inv=OCIO::Config::CreateRaw()->getProcessor(group)->getDefaultCPUProcessor();
    if(argc==3) {
        std::ifstream input(argv[1],std::ios::binary|std::ios::ate);
        if(!input)throw std::runtime_error("cannot open inverse input");
        auto bytes=input.tellg();if(bytes<0 || bytes%sizeof(float))throw std::runtime_error("invalid inverse input length");
        std::vector<float> values(static_cast<size_t>(bytes)/sizeof(float));input.seekg(0);input.read(reinterpret_cast<char *>(values.data()),bytes);
        RGB high{64.f,64.f,64.f};forward->applyRGB(high.data());float ceiling=(high[0]+high[1]+high[2])/3.f;
        std::vector<RGB> rgb;rgb.reserve(values.size());
        for(float value:values) {if(!std::isfinite(value)||value<0.f||value>ceiling)throw std::runtime_error("inverse requested outside the declared neutral study domain");rgb.push_back({value,value,value});}
        OCIO::PackedImageDesc image(rgb.data(),static_cast<long>(rgb.size()),1,3);inv->apply(image);
        for(size_t i=0;i<values.size();i++) {values[i]=(rgb[i][0]+rgb[i][1]+rgb[i][2])/3.f;if(!std::isfinite(values[i])||values[i]<0.f)throw std::runtime_error("invalid neutral inverse output");}
        std::ofstream output(argv[2],std::ios::binary);output.write(reinterpret_cast<const char *>(values.data()),static_cast<std::streamsize>(values.size()*sizeof(float)));
        if(!output)throw std::runtime_error("cannot write inverse output");
        return 0;
    }
    if(argc!=1)throw std::runtime_error("usage: drt-lift-neutral [input.f32 output.f32]");
    std::ofstream csv(".cache/probes/drt-lift/neutral.csv");csv<<"scene,display,inverse_scene,roundtrip_display\n";csv.precision(17);
    float prev=-1.f, max_roundtrip=0.f;double max_inverse_relative=0.;unsigned violations=0;
    std::printf("OCIO %s; actual Antitone sRGB/D65 100nit neutral forward; public ACES2 inverse plus AP0->AP1; scalar study only\n",OCIO::GetVersion());
    for(int index=0;index<=9728;index++) {
        const double scene=std::exp2(-32.+double(index)/256.); // up to 64: finite, monotonic inverse-study domain
        RGB out{float(scene),float(scene),float(scene)};forward->applyRGB(out.data());
        const float display=(out[0]+out[1]+out[2])/3.f;
        RGB reconstructed{display,display,display};inv->applyRGB(reconstructed.data());
        double neutral=(double(reconstructed[0])+reconstructed[1]+reconstructed[2])/3.;
        auto roundtrip=reconstructed;forward->applyRGB(roundtrip.data());
        const float back=(roundtrip[0]+roundtrip[1]+roundtrip[2])/3.f;
        max_roundtrip=std::max(max_roundtrip,std::abs(back-display));
        max_inverse_relative=std::max(max_inverse_relative,std::abs(neutral/scene-1.));
        if(display<=prev)violations++;prev=display;
        csv<<scene<<','<<display<<','<<neutral<<','<<back<<'\n';
        if(index%256==0)std::printf("EV=%g scene=%.9g display=%.9g public_inverse=%.9g back=%.9g\n",-32.+double(index)/256.,scene,display,neutral,back);
    }
    std::printf("nodes=9729 monotonic_violations=%u public_inverse_max_relative_scene_error=%.9g max_display_roundtrip_error=%.9g\n",violations,max_inverse_relative,max_roundtrip);
    // Verify target neutrality independently: the working lift is fixed, not selected by output target.
    auto p3=antitone::rendering(1)->getDefaultCPUProcessor();
    double difference=0.;
    for(int i=0;i<=256;i++) {float e=std::exp2(-16.f+float(i)/16.f);RGB a{e,e,e},b=a;forward->applyRGB(a.data());p3->applyRGB(b.data());difference=std::max(difference,std::abs((double(a[0])+a[1]+a[2]-b[0]-b[1]-b[2])/3.));}
    std::printf("sRGB_P3_neutral_max_absolute_difference=%.9g\n",difference);
    return violations?1:0;
} catch(const std::exception &e){std::fprintf(stderr,"%s\n",e.what());return 2;}
