#include <OpenColorIO/OpenColorIO.h>
#include <array>
#include <cstdio>
namespace OCIO=OCIO_NAMESPACE;
int main() try {
 const char* names[]={"sRGB D65","P3 D65","Rec2020 D65","ProPhoto D50","ProPhoto D65","ProPhoto D60"};
 const std::array<double,9> targets[]={
 {100,.64,.33,.30,.60,.15,.06,.3127,.3290},
 {100,.68,.32,.265,.69,.15,.06,.3127,.3290},
 {100,.708,.292,.170,.797,.131,.046,.3127,.3290},
 {100,.7347,.2653,.1596,.8404,.0366,.0001,.3457,.3585},
 {100,.7347,.2653,.1596,.8404,.0366,.0001,.3127,.3290},
 {100,.7347,.2653,.1596,.8404,.0366,.0001,.32168,.33767}};
 for(unsigned i=0;i<6;++i) {
  auto group=OCIO::GroupTransform::Create(); auto basis=OCIO::BuiltinTransform::Create();basis->setStyle("ACEScg_to_ACES2065-1");group->appendTransform(basis);
  group->appendTransform(OCIO::FixedFunctionTransform::Create(OCIO::FIXED_FUNCTION_ACES_OUTPUT_TRANSFORM_20,targets[i].data(),9));
  auto cpu=OCIO::Config::CreateRaw()->getProcessor(group)->getDefaultCPUProcessor();
  for(const auto& input:std::array<std::array<float,4>,6>{{{.4f,.4f,.2f,1},{.4f,.2f,.2f,1},{.2f,.4f,.4f,1},{.4f,.4f,.4f,1},{.18f,.18f,0.f,1},{.2f,.2f,.01f,1}}}) {auto pixel=input;cpu->applyRGBA(pixel.data());std::printf("%s AP1=(%.2f %.2f %.2f) native_linear=(%.9g %.9g %.9g)\n",names[i],input[0],input[1],input[2],pixel[0],pixel[1],pixel[2]);}
 }
} catch(const std::exception& e) {std::fprintf(stderr,"%s\n",e.what());return 1;}
