
// Declaration of all textures

layout(set=0, binding = 1) uniform sampler2D aces_reach_m_table_0Sampler; 
layout(set=0, binding = 2) uniform sampler2D aces_gamut_cusp_table_0Sampler; 

// Declaration of all helper methods

float aces_reach_m_table_0_sample(float h)
{
  float i_base = floor(h);
  float i_lo = i_base + float(1);
  float i_hi = i_lo + 1.0;
  float lo = texture(aces_reach_m_table_0Sampler, vec2((i_lo + 0.5) / float (363), 0.0)).r;
  float hi = texture(aces_reach_m_table_0Sampler, vec2((i_hi + 0.5) / float (363), 0.5)).r;
  float t = h - i_base;
  return mix(lo, hi, t);
}
float aces_tonescale_fwd0(float J)
{
  float A = 0.0323680267 * pow(abs(J) * 0.00999999978, 0.879464149);
  float Y = pow(( 27.1299992 * A) / (1.0f - A), 2.3809523809523809);
  float f = 41.1687241 * pow(Y / (Y + 14.433662900070772), 1.14999998);
  float Y_ts = max(0.0, f * f / (f + 0.0399999991));
  float F_L_Y = pow(0.79370057210326195 * Y_ts, 0.42);
  float J_ts = 100. * pow((F_L_Y / ( 27.1299992 + F_L_Y)) * 30.8946857, 1.13705599);
  return sign(J) * J_ts;
}
float aces_toe_fwd0(float x, float limit, float k1_in, float k2_in)
{
  float k2 = max(k2_in, 0.001);
  float k1 = sqrt(k1_in * k1_in + k2 * k2);
  float k3 = (limit + k1) / (limit + k2);
  return (x > limit) ? x : 0.5 * (k3 * x - k1 + sqrt((k3 * x - k1) * (k3 * x - k1) + 4.0 * k2 * k3 * x));
}
const float aces_gamut_cusp_table_0_hues_array[363] = float[363](-0.988128662, 0., 1.00394082, 2.00788164, 3.01182246, 4.01576328, 5.01970387, 6.02364492, 7.02758598, 8.03152657, 9.03546715, 10.0394077, 11.0433493, 12.0472898, 13.0512304, 14.055172, 15.0591125, 16.0630531, 17.0669937, 18.0709343, 19.0748749, 20.0788155, 21.0827579, 22.0866985, 23.0906391, 24.0945797, 25.0912819, 26.087986, 27.0846882, 28.0854397, 29.0861912, 30.0869427, 31.0876961, 32.0884476, 33.0891991, 34.0899506, 35.0907021, 36.0914536, 37.092205, 38.0929565, 39.093708, 40.0944595, 41.095211, 42.0959625, 43.0967178, 44.0974693, 45.0982208, 46.0989723, 47.0997238, 48.1004753, 49.1012268, 50.1019783, 51.1027298, 52.1034813, 53.1042328, 54.1049843, 55.1057358, 56.1064873, 57.1072388, 58.1079903, 59.1087418, 60.1094971, 61.1102486, 62.1110001, 63.1117516, 64.1125031, 65.1132507, 66.114006, 67.1147614, 68.115509, 69.1162643, 70.117012, 71.1177673, 72.118515, 73.1192703, 74.120018, 75.1207733, 76.121521, 77.1222763, 78.123024, 79.1237793, 80.124527, 81.1252823, 82.12603, 83.1267853, 84.1275406, 85.1282883, 86.1290436, 87.1297913, 88.1305466, 89.1312943, 90.1320496, 91.1327972, 92.1335526, 93.1343002, 94.1350555, 95.1358032, 96.1365585, 97.1373062, 98.1380615, 99.1388092, 100.139565, 101.14032, 102.141068, 103.141823, 104.14257, 105.143326, 106.144073, 106.575989, 107.579819, 108.583641, 109.587471, 110.591293, 111.595123, 112.598953, 113.602776, 114.606606, 115.610428, 116.614258, 117.618088, 118.62191, 119.62574, 120.629562, 121.633392, 122.637222, 123.641045, 124.644875, 125.648697, 126.652527, 127.656349, 128.660187, 129.664001, 130.667831, 131.671661, 132.675491, 133.679321, 134.683136, 135.686966, 136.690796, 137.694626, 138.698456, 139.702271, 140.7061, 141.70993, 142.71376, 143.717575, 144.721405, 145.725235, 147.194794, 148.212769, 149.230759, 150.248749, 151.266724, 152.284698, 153.302689, 154.320679, 155.338654, 156.356628, 157.374619, 158.392609, 159.410583, 160.428558, 161.446548, 162.464539, 163.482513, 164.500488, 165.518478, 166.536469, 167.554443, 168.572418, 169.590408, 170.608398, 171.626373, 172.644348, 173.662338, 174.680328, 175.698303, 176.716278, 177.734268, 178.752258, 179.770233, 180.788208, 181.806198, 182.824188, 183.842163, 184.860138, 185.878128, 186.896118, 187.914093, 188.932068, 189.950058, 190.968048, 191.986023, 192.085907, 193.089325, 194.092743, 195.096146, 196.099564, 197.102982, 198.106384, 199.109802, 200.11322, 201.116638, 202.120056, 203.123459, 204.126877, 205.130295, 206.133698, 207.137115, 208.140533, 209.143951, 210.147369, 211.150772, 212.15419, 213.157608, 214.161011, 215.164429, 216.167847, 217.171265, 218.174683, 219.178085, 220.181503, 221.184921, 222.188324, 223.191742, 224.19516, 225.198578, 226.201996, 227.205399, 228.208817, 229.212234, 230.215637, 231.219055, 232.222473, 233.225891, 234.229309, 235.232712, 236.23613, 237.239548, 238.24295, 239.246368, 240.249786, 241.253204, 242.256622, 243.260025, 244.263443, 245.266861, 246.270264, 247.273682, 248.2771, 249.280518, 250.283936, 251.287338, 252.290756, 253.294174, 254.297577, 255.300995, 256.304413, 257.307831, 258.311249, 259.314667, 260.318054, 261.321472, 262.32489, 263.328308, 264.331726, 265.335144, 266.296783, 267.258453, 268.220093, 269.228455, 270.236786, 271.245148, 272.25351, 273.261871, 274.270203, 275.278564, 276.286926, 277.295258, 278.303619, 279.311981, 280.320343, 281.328674, 282.337036, 283.345398, 284.353729, 285.362091, 286.370453, 287.378815, 288.387146, 289.395508, 290.40387, 291.412201, 292.420563, 293.428925, 294.437256, 295.445618, 296.453979, 297.462341, 298.470673, 299.479034, 300.487396, 301.495728, 302.504089, 303.512451, 304.520813, 305.529144, 306.537506, 307.545868, 308.554199, 309.562561, 310.570923, 311.579285, 312.587616, 313.595978, 314.60434, 315.612671, 316.621033, 317.629395, 318.637756, 319.646088, 320.654449, 321.662811, 322.671143, 323.679504, 324.687866, 325.696228, 326.704559, 327.712921, 328.721283, 329.729614, 330.737976, 331.746338, 332.7547, 333.763031, 334.771393, 335.779755, 336.788086, 337.796448, 338.80481, 339.813141, 340.821503, 341.829865, 342.213898, 343.202026, 344.190125, 345.178253, 346.166351, 347.15448, 348.142609, 349.130707, 350.118835, 351.106964, 352.095062, 353.083191, 354.071289, 355.059418, 356.047546, 357.035645, 358.023773, 359.011871, 360., 361.003937);
vec3 aces_gamut_cusp_table_0_sample(float h)
{
  int i = int(h) + 1;
  int i_lo = int(max(float(0), float(i + 0)));
  int i_hi = int(min(float(361), float(i + 2)));
  while (i_lo + 1 < i_hi)
  {
    float hcur = aces_gamut_cusp_table_0_hues_array[i];
    if (h > hcur)
    {
      i_lo = i;
    }
    else
    {
      i_hi = i;
    }
    i = (i_lo + i_hi) / 2;
  }
  vec3 lo = texture(aces_gamut_cusp_table_0Sampler, vec2((float(i_hi) - 1.0 + 0.5) / float(363), 0.5)).rgb;
  vec3 hi = texture(aces_gamut_cusp_table_0Sampler, vec2((float(i_hi) + 0.5) / float(363), 0.5)).rgb;
  float t = (h - aces_gamut_cusp_table_0_hues_array[i_hi - 1]) / (aces_gamut_cusp_table_0_hues_array[i_hi] - aces_gamut_cusp_table_0_hues_array[i_hi - 1]);
  return mix(lo, hi, t);
}
float aces_get_focus_gain0(float J, float cuspJ)
{
  float thr = mix(cuspJ, 511.809357, 0.300000);
  if (J > thr)
  {
    float gain = ( 511.809357 - thr) / max(0.0001, 511.809357 - J);
    gain = log(gain)/log(10.0);
    return gain * gain + 1.0;
  }
  else
  {
    return 1.0;
  }
}
float aces_solve_J_intersect0(float J, float M, float focusJ, float slope_gain)
{
  float M_scaled = M / slope_gain;
  float a = M_scaled / focusJ;
  if (J < focusJ)
  {
    float b = 1.0 - M_scaled;
    float c = -J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b + root);
  }
  else
  {
    float b = - (1.0 + M_scaled + 511.809357 * a);
    float c = 511.809357 * M_scaled + J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b - root);
  }
}
float aces_find_gamut_boundary_intersection0(vec2 JM_cusp, float gamma_top_inv, float gamma_bottom_inv, float J_intersect_source, float J_intersect_cusp, float slope)
{
  float M_boundary_lower = J_intersect_cusp * pow(J_intersect_source / J_intersect_cusp, gamma_bottom_inv) / (JM_cusp.r / JM_cusp.g - slope);
  float M_boundary_upper = JM_cusp.g * (511.809357 - J_intersect_cusp) * pow((511.809357 - J_intersect_source) / (511.809357 - J_intersect_cusp), gamma_top_inv) / (slope * JM_cusp.g + 511.809357 - JM_cusp.r);
  float smin = 0.0;
  {
    float a = M_boundary_lower;
    float b = M_boundary_upper;
    float s = 0.119999997 * JM_cusp.g;
    float h = max(s - abs(a - b), 0.0) / s;
    smin = min(a, b) - h * h * h * s * 0.16666666666666666;
  }
  return smin;
}
float aces_remap_M_fwd0(float M, float gamut_boundary_M, float reach_boundary_M)
{
  float boundary_ratio = gamut_boundary_M / reach_boundary_M;
  float proportion = max(boundary_ratio, 0.75);
  float threshold = proportion * gamut_boundary_M;
  if (proportion >= 1.0f || M <= threshold)
  {
    return M;
  }
  float m_offset = M - threshold;
  float gamut_offset = gamut_boundary_M - threshold;
  float reach_offset = reach_boundary_M - threshold;
  float scale = reach_offset / ((reach_offset / gamut_offset) - 1.0f);
  float nd = m_offset / scale;
  return threshold + scale * nd / (1.0f + nd);
}
vec3 aces_gamut_compress0(vec3 JMh, float Jx, vec3 JMGcusp, float reachMaxM)
{
  float J = JMh.r;
  float M = JMh.g;
  float h = JMh.b;
  if (M <= 0.0 || J > 511.809357)
  {
    return vec3(J, 0.0, h);
  }
  else
  {
    vec2 JMcusp = JMGcusp.rg;
    float focusJ = mix(JMcusp.r, 44.315868, min(1.0, 1.300000 - (JMcusp.r / 511.809357)));
    float slope_gain = 2628.07275 * aces_get_focus_gain0(Jx, JMcusp.r);
    float J_intersect_source = aces_solve_J_intersect0(JMh.r, JMh.g, focusJ, slope_gain);
    float gamut_slope = (J_intersect_source < focusJ) ? J_intersect_source : (511.809357 - J_intersect_source);
    gamut_slope = gamut_slope * (J_intersect_source - focusJ) / (focusJ * slope_gain);
    float gamma_top_inv = JMGcusp.b;
    float gamma_bottom_inv = 0.798630059;
    float J_intersect_cusp = aces_solve_J_intersect0(JMcusp.r, JMcusp.g, focusJ, slope_gain);
    float gamutBoundaryM = aces_find_gamut_boundary_intersection0(JMcusp, gamma_top_inv, gamma_bottom_inv, J_intersect_source, J_intersect_cusp, gamut_slope);
    if (gamutBoundaryM <= 0.0)
    {
      return vec3(J, 0.0, h);
    }
    float reachBoundaryM = 511.809357 * pow(J_intersect_source / 511.809357,  0.879464149);
    reachBoundaryM = reachBoundaryM / ((511.809357 / reachMaxM) - gamut_slope);
    float remapped_M = aces_remap_M_fwd0(M, gamutBoundaryM, reachBoundaryM);
    float remapped_J = J_intersect_source + remapped_M * gamut_slope;
    return vec3(remapped_J, remapped_M, h);
  }
}

// Declaration of the OCIO shader function

vec4 antitone_render(vec4 inPixel)
{
  vec4 outColor = inPixel;
  
  // Add Range processing
  
  {
    outColor.rgb = max(vec3(0., 0., 0.), outColor.rgb);
    outColor.rgb = min(vec3(1024., 1024., 1024.), outColor.rgb);
  }
  
  // Add Matrix processing
  
  {
    vec4 res = vec4(outColor.rgb.r, outColor.rgb.g, outColor.rgb.b, outColor.a);
    vec4 tmp = res;
    res = mat4(0.69545224135745187, 0.044794563372037723, -0.0055258825581135443, 0., 0.14067869647029416, 0.85967111845642175, 0.0040252103059786595, 0., 0.16386906217225405, 0.0955343181715404, 1.0015006722521349, 0., 0., 0., 0., 1.) * tmp;
    outColor.rgb = vec3(res.x, res.y, res.z);
    outColor.a = res.w;
  }
  
  // Add FixedFunction 'ACES_OutputTransform20 (Forward)' processing
  
  {
    
    // Add RGB to JMh
    
    vec3 JMh;
    vec3 Aab;
    {
      {
        vec3 lms = mat3(0.445181042, 0.123734146, 0.0117007261, 0.34964928, 0.613643706, 0.0280607939, -0.00112973328, 0.0563228093, 0.753939092) * outColor.rgb;
        vec3 F_L_v = pow(abs(lms), vec3(0.419999987, 0.419999987, 0.419999987));
        vec3 rgb_a = (sign(lms) * F_L_v) / ( 27.1299992 + F_L_v);
        Aab = mat3(20.2588081, 15480., 1720., 10.1294041, -16887.2734, 1720., 0.506470203, 1407.27271, -3440.) * rgb_a.rgb;
      }
      {
        if (Aab.r <= 0.0)
        {
          JMh.rgb = vec3(0., 0., 0.);
        }
        else
        {
          float J = 100. * pow(Aab.r, 1.13705599);
          float M = (J == 0.0) ? 0.0 : sqrt(Aab.g * Aab.g + Aab.b * Aab.b);
          float h = (Aab.g == 0.0) ? 0.0 : atan(Aab.b, Aab.g) * 57.29577951308238;
          h = h - floor(h / 360.0) * 360.0;
          h = (h < 0.0) ? h + 360.0 : h;
          JMh.rgb = vec3(J, M, h);
        }
      }
      outColor.rgb = JMh;
    }
    float h_rad = outColor.b * 0.0174532924;
    float cos_hr = cos(h_rad);
    float sin_hr = sin(h_rad);
    
    // Add ToneScale and ChromaCompress (fwd)
    
    float J_ts = aces_tonescale_fwd0(outColor.r);
    // Sample tables (fwd)
    float reachMaxM = aces_reach_m_table_0_sample(outColor.b);
    
    {
      float J = outColor.r;
      float M = outColor.g;
      float h = outColor.b;
      float M_cp = M;
      if (M != 0.0)
      {
        float nJ = J_ts / 511.809357;
        float snJ = max(0.0, 1.0 - nJ);
        float Mnorm;
        {
          float cos_hr2 = 2.0 * cos_hr * cos_hr - 1.0;
          float sin_hr2 = 2.0 * cos_hr * sin_hr;
          float cos_hr3 = 4.0 * cos_hr * cos_hr * cos_hr - 3.0 * cos_hr;
          float sin_hr3 = 3.0 * sin_hr - 4.0 * sin_hr * sin_hr * sin_hr;
          vec3 cosines = vec3(cos_hr, cos_hr2, cos_hr3);
          vec3 cosine_weights = vec3(45.766968641166685, 66.462777396998405, 31.816112854671477);
          vec3 sines = vec3(sin_hr, sin_hr2, sin_hr3);
          vec3 sine_weights = vec3(59.180157222046851, -25.71601346775055, 37.102144623813629);
          Mnorm = dot(cosines, cosine_weights) + dot(sines, sine_weights) + 311.26407262023929;
        }
        float limit = pow(nJ, 0.879464149) * reachMaxM / Mnorm;
        M_cp = M * pow(J_ts / J, 0.879464149);
        M_cp = M_cp / Mnorm;
        M_cp = limit - aces_toe_fwd0(limit - M_cp, limit - 0.001, snJ * 0.200000003, sqrt(nJ * nJ + 0.000125000006));
        M_cp = aces_toe_fwd0(M_cp, limit, nJ * 15.088315, snJ);
        M_cp = M_cp * Mnorm;
      }
      outColor.rgb = vec3(J_ts, M_cp, h);
    }
    
    // Add GamutCompress (fwd)
    
    {
      vec3 JMGcusp = aces_gamut_cusp_table_0_sample(outColor.b);
      outColor.rgb = aces_gamut_compress0(outColor.rgb, outColor.r, JMGcusp, reachMaxM);
    }
    
    // Add JMh to RGB
    
    {
      vec3 JMh = outColor.rgb;
      vec3 Aab;
      {
        Aab.r = pow(JMh.r * 0.00999999978, 0.879464149);
        Aab.g = JMh.g * cos_hr;
        Aab.b = JMh.g * sin_hr;
      }
      {
        vec3 rgb_a = mat3(0.0323680304, 0.0323680304, 0.0323680304, 2.07657631e-05, -4.10250432e-05, -1.01296409e-05, 1.3260621e-05, -1.20174373e-05, -0.000290076074) * Aab.rgb;
        vec3 rgb_a_lim = min( abs(rgb_a), vec3(0.99000001, 0.99000001, 0.99000001) );
        vec3 lms = sign(rgb_a) * pow( 27.1299992 * rgb_a_lim / (1.0f - rgb_a_lim), vec3(2.38095236, 2.38095236, 2.38095236));
        JMh.rgb = mat3(4.18920946, -0.841454685, 0.00181781233, -2.83307672, 2.44025636, -0.111337893, -0.0962110385, -0.338880897, 1.36944115) * lms;
      }
      outColor.rgb = JMh;
    }
  }
  
  // Add Range processing
  
  {
    outColor.rgb = max(vec3(0., 0., 0.), outColor.rgb);
    outColor.rgb = min(vec3(40., 40., 40.), outColor.rgb);
  }

  return outColor;
}
